use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ap_model::{PlaylistType, SourceType, Track, dedupe};
use ap_store::{Db, catalog as cache, playlists as store};
use futures_util::{StreamExt, stream};
use tokio::sync::{Mutex, Semaphore, broadcast, watch};

use crate::{CatalogRepository, LibraryError, Result, catalog::normalize_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSelection {
    pub tracks: Vec<Track>,
    pub start_index: usize,
}

#[derive(Clone)]
pub struct PlaylistRepository {
    inner: Arc<Inner>,
}
struct Inner {
    db: Db,
    catalog: CatalogRepository,
    playlists: watch::Sender<Vec<store::PlaylistRecord>>,
    tracks: Mutex<HashMap<String, watch::Sender<Vec<Track>>>>,
    changes: broadcast::Sender<String>,
    fetches: Semaphore,
}

impl PlaylistRepository {
    pub async fn new(db: Db, catalog: CatalogRepository) -> Result<Self> {
        let rows = db.read(store::list_playlists).await?;
        let (playlists, _) = watch::channel(rows);
        let (changes, _) = broadcast::channel(128);
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                catalog,
                playlists,
                tracks: Mutex::new(HashMap::new()),
                changes,
                fetches: Semaphore::new(4),
            }),
        })
    }
    pub fn observe_playlists(&self) -> watch::Receiver<Vec<store::PlaylistRecord>> {
        self.inner.playlists.subscribe()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.changes.subscribe()
    }
    pub async fn list(&self) -> Result<Vec<store::PlaylistRecord>> {
        Ok(self.inner.db.read(store::list_playlists).await?)
    }
    pub async fn get(&self, id: &str) -> Result<Option<store::PlaylistRecord>> {
        let id = id.to_owned();
        Ok(self
            .inner
            .db
            .read(move |c| store::get_playlist(c, &id))
            .await?)
    }
    /// Returns saved rows/materialization only. Never calls providers.
    pub async fn cached_tracks(&self, id: &str) -> Result<Vec<Track>> {
        let id = id.to_owned();
        Ok(self
            .inner
            .db
            .read(
                move |c| match store::get_playlist(c, &id)?.map(|p| p.playlist_type) {
                    Some(PlaylistType::Standard) => saved_tracks(c, &id),
                    Some(PlaylistType::Union) => {
                        Ok(cache::tracks(c, &cache::union_owner_key(&id))?
                            .map_or_else(Vec::new, |t| t.tracks))
                    }
                    None => Ok(vec![]),
                },
            )
            .await?)
    }
    pub async fn observe_tracks(&self, id: &str) -> Result<watch::Receiver<Vec<Track>>> {
        // Hold the subscription registry across the cache read so publication cannot be lost.
        let mut senders = self.inner.tracks.lock().await;
        if let Some(sender) = senders.get(id) {
            return Ok(sender.subscribe());
        }
        let (sender, receiver) = watch::channel(self.cached_tracks(id).await?);
        senders.insert(id.to_owned(), sender);
        Ok(receiver)
    }
    pub async fn reload(&self) -> Result<()> {
        self.inner.playlists.send_replace(self.list().await?);
        let mut senders = self.inner.tracks.lock().await;
        for (id, sender) in senders.iter_mut() {
            sender.send_replace(self.cached_tracks(id).await?);
        }
        let _ = self.inner.changes.send(String::new());
        Ok(())
    }
    async fn changed(&self, id: &str) -> Result<()> {
        self.inner.playlists.send_replace(self.list().await?);
        let senders = self.inner.tracks.lock().await;
        if let Some(sender) = senders.get(id) {
            sender.send_replace(self.cached_tracks(id).await?);
        }
        let _ = self.inner.changes.send(id.to_owned());
        Ok(())
    }
    pub async fn create(&self, name: &str, kind: PlaylistType) -> Result<store::PlaylistRecord> {
        let new = store::NewPlaylist::new(
            if name.trim().is_empty() {
                "Untitled Playlist"
            } else {
                name
            },
            kind,
        );
        let row = self
            .inner
            .db
            .call(move |c| store::create_playlist(c, &new))
            .await?;
        self.changed(&row.id).await?;
        Ok(row)
    }
    pub async fn update(
        &self,
        id: &str,
        patch: store::PlaylistPatch,
    ) -> Result<Option<store::PlaylistRecord>> {
        let owned = id.to_owned();
        let row = self
            .inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let changed = store::update_playlist(c, &owned, &patch)?;
                    clear_timestamp(c, &owned)?;
                    Ok(changed)
                })
            })
            .await?;
        self.changed(id).await?;
        Ok(row)
    }
    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        self.update(
            id,
            store::PlaylistPatch {
                name: Some(if name.trim().is_empty() {
                    String::new()
                } else {
                    name.to_owned()
                }),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let owned = id.to_owned();
        let deleted = self
            .inner
            .db
            .call(move |c| store::delete_playlist(c, &owned))
            .await?;
        self.changed(id).await?;
        Ok(deleted)
    }
    pub async fn set_distinct(&self, id: &str, distinct: bool) -> Result<()> {
        let owned = id.to_owned();
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    store::set_playlist_distinct(c, &owned, distinct)?;
                    clear_timestamp(c, &owned)
                })
            })
            .await?;
        self.changed(id).await
    }
    pub async fn add_tracks(
        &self,
        id: &str,
        tracks: Vec<Track>,
    ) -> Result<Vec<store::PlaylistTrackRecord>> {
        let owned = id.to_owned();
        let rows = self
            .inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let rows = store::add_tracks(c, &owned, &tracks)?;
                    for (row, track) in rows.iter().zip(&tracks) {
                        c.execute(
                            "UPDATE playlist_tracks SET url=?2 WHERE id=?1",
                            ap_store::rusqlite::params![
                                row.id,
                                track.url.as_deref().map(ap_store::strip_plex_token)
                            ],
                        )?;
                    }
                    clear_timestamp(c, &owned)?;
                    Ok(rows)
                })
            })
            .await?;
        self.changed(id).await?;
        Ok(rows)
    }
    pub async fn track_rows(&self, id: &str) -> Result<Vec<store::PlaylistTrackRecord>> {
        let id = id.to_owned();
        Ok(self
            .inner
            .db
            .read(move |c| store::list_tracks(c, &id))
            .await?)
    }
    pub async fn remove_track(&self, id: &str, row_id: i64) -> Result<bool> {
        let owned = id.to_owned();
        let removed = self
            .inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let removed = store::remove_track(c, &owned, row_id)?;
                    if removed {
                        clear_timestamp(c, &owned)?;
                    }
                    Ok(removed)
                })
            })
            .await?;
        self.changed(id).await?;
        Ok(removed)
    }
    pub async fn remove_track_at(&self, id: &str, index: usize) -> Result<bool> {
        let owned = id.to_owned();
        let removed = self
            .inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let rows = store::list_tracks(c, &owned)?;
                    let removed = match rows.get(index) {
                        Some(row) => store::remove_track(c, &owned, row.id)?,
                        None => false,
                    };
                    if removed {
                        clear_timestamp(c, &owned)?;
                    }
                    Ok(removed)
                })
            })
            .await?;
        self.changed(id).await?;
        Ok(removed)
    }
    /// Unknown/duplicate requested ids are ignored; unmentioned rows retain their relative order.
    pub async fn reorder_tracks(&self, id: &str, order: Vec<i64>) -> Result<()> {
        let owned = id.to_owned();
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let rows = store::list_tracks(c, &owned)?;
                    let ids = ordered_ids(rows.iter().map(|r| r.id), &order);
                    for (position, row) in ids.into_iter().enumerate() {
                        store::move_track(
                            c,
                            &owned,
                            row,
                            i64::try_from(position).unwrap_or(i64::MAX),
                        )?;
                    }
                    clear_timestamp(c, &owned)
                })
            })
            .await?;
        self.changed(id).await
    }
    pub async fn union_sources(&self, id: &str) -> Result<Vec<store::UnionSourceRecord>> {
        let id = id.to_owned();
        Ok(self
            .inner
            .db
            .read(move |c| store::list_union_sources(c, &id))
            .await?)
    }
    pub async fn replace_union_sources(
        &self,
        id: &str,
        sources: Vec<(SourceType, String)>,
    ) -> Result<()> {
        let sources = sources
            .into_iter()
            .map(|(source_type, source_playlist_id)| store::NewUnionSource {
                source_type,
                source_playlist_id: normalize_id(source_type, &source_playlist_id),
                added_at: ap_store::now_secs(),
            })
            .collect::<Vec<_>>();
        let owned = id.to_owned();
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    store::replace_union_sources(c, &owned, &sources)?;
                    invalidate_union(c, &owned)
                })
            })
            .await?;
        self.changed(id).await?;
        self.materialize_union(id, false).await?;
        Ok(())
    }
    pub async fn add_union_source(
        &self,
        id: &str,
        source: SourceType,
        source_id: &str,
    ) -> Result<()> {
        let owned = id.to_owned();
        let source_id = normalize_id(source, source_id);
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    store::add_union_source(c, &owned, source, &source_id)?;
                    invalidate_union(c, &owned)
                })
            })
            .await?;
        self.changed(id).await?;
        self.materialize_union(id, false).await?;
        Ok(())
    }
    pub async fn remove_union_source(&self, id: &str, row_id: i64) -> Result<()> {
        let owned = id.to_owned();
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    store::remove_union_source(c, &owned, row_id)?;
                    invalidate_union(c, &owned)
                })
            })
            .await?;
        self.changed(id).await?;
        self.materialize_union(id, false).await?;
        Ok(())
    }
    pub async fn reorder_union_sources(&self, id: &str, order: Vec<i64>) -> Result<()> {
        let owned = id.to_owned();
        self.inner
            .db
            .call(move |c| {
                ap_store::in_tx(c, |c| {
                    let rows = store::list_union_sources(c, &owned)?;
                    for (position, row) in ordered_ids(rows.iter().map(|r| r.id), &order)
                        .into_iter()
                        .enumerate()
                    {
                        store::move_union_source(
                            c,
                            &owned,
                            row,
                            i64::try_from(position).unwrap_or(i64::MAX),
                        )?;
                    }
                    invalidate_union(c, &owned)
                })
            })
            .await?;
        self.changed(id).await?;
        self.materialize_union(id, false).await?;
        Ok(())
    }
    pub async fn materialize_union(&self, id: &str, force: bool) -> Result<Vec<Track>> {
        let mut force = force;
        loop {
            if self
                .get(id)
                .await?
                .is_none_or(|p| p.playlist_type != PlaylistType::Union)
            {
                return Ok(vec![]);
            }
            let sources = self.union_sources(id).await?;
            // Expand CUSTOM unions exactly one level, before the globally bounded network stage.
            let mut expanded = vec![];
            for source in &sources {
                if source.source_type == SourceType::Custom
                    && self
                        .get(&source.source_playlist_id)
                        .await?
                        .is_some_and(|p| p.playlist_type == PlaylistType::Union)
                {
                    expanded.extend(self.union_sources(&source.source_playlist_id).await?);
                } else {
                    expanded.push(source.clone());
                }
            }
            let batches: Vec<_> = stream::iter(expanded)
                .map(|source| async move {
                    match source.source_type {
                        SourceType::Custom => {
                            let id = source.source_playlist_id;
                            self.inner
                                .db
                                .read(move |c| saved_tracks(c, &id))
                                .await
                                .unwrap_or_default()
                        }
                        SourceType::All => vec![],
                        _ => {
                            let _permit = self
                                .inner
                                .fetches
                                .acquire()
                                .await
                                .expect("private semaphore is never closed");
                            self.inner
                                .catalog
                                .tracks(source.source_type, &source.source_playlist_id, force)
                                .await
                                .unwrap_or_default()
                        }
                    }
                })
                .buffered(4)
                .collect()
                .await;
            let tracks = transform(batches.into_iter().flatten().collect(), |tracks| {
                let mut seen = HashSet::new();
                tracks
                    .into_iter()
                    .filter(|t| seen.insert((t.source, t.id.clone())))
                    .map(|mut t| {
                        t.enriched.get_or_insert(true);
                        t
                    })
                    .collect::<Vec<_>>()
            })
            .await?;
            let stored = tracks.clone();
            let owned = id.to_owned();
            let saved = self
                .inner
                .db
                .call(move |c| {
                    ap_store::in_tx(c, |c| {
                        if store::get_playlist(c, &owned)?
                            .is_none_or(|p| p.playlist_type != PlaylistType::Union)
                            || store::list_union_sources(c, &owned)? != sources
                        {
                            return Ok(false);
                        }
                        cache::replace_tracks(
                            c,
                            &cache::union_owner_key(&owned),
                            &stored,
                            ap_store::now_ms(),
                        )?;
                        c.execute(
                            "UPDATE custom_playlists SET track_count = ?2 WHERE id = ?1",
                            ap_store::rusqlite::params![
                                owned,
                                i64::try_from(stored.len()).unwrap_or(i64::MAX)
                            ],
                        )?;
                        Ok(true)
                    })
                })
                .await?;
            if saved {
                self.changed(id).await?;
                return Ok(tracks);
            }
            force = false;
        }
    }
    pub async fn playback_selection(
        &self,
        id: &str,
        from_index: Option<usize>,
    ) -> Result<PlaybackSelection> {
        let Some(playlist) = self.get(id).await? else {
            return Ok(PlaybackSelection {
                tracks: vec![],
                start_index: 0,
            });
        };
        let tracks = if playlist.playlist_type == PlaylistType::Union {
            self.materialize_union(id, false).await?
        } else {
            self.cached_tracks(id).await?
        };
        selection(tracks, from_index, playlist.is_distinct).await
    }
    pub async fn provider_playback_selection(
        &self,
        source: SourceType,
        id: &str,
        from_index: Option<usize>,
    ) -> Result<PlaybackSelection> {
        let tracks = self.inner.catalog.tracks(source, id, false).await?;
        selection(
            tracks,
            from_index,
            self.inner.catalog.is_distinct(source, id).await?,
        )
        .await
    }
    pub async fn duplicate_groups(&self, id: &str) -> Result<Vec<dedupe::DuplicateGroup>> {
        let tracks = self.cached_tracks(id).await?;
        transform(tracks, |tracks| {
            dedupe::deduplicate_tracks(tracks).duplicate_groups
        })
        .await
    }
}

fn ordered_ids(existing: impl Iterator<Item = i64>, requested: &[i64]) -> Vec<i64> {
    let existing: Vec<_> = existing.collect();
    let valid: HashSet<_> = existing.iter().copied().collect();
    let mut seen = HashSet::new();
    requested
        .iter()
        .copied()
        .chain(existing)
        .filter(|id| valid.contains(id) && seen.insert(*id))
        .collect()
}
async fn transform<T: Send + 'static>(
    tracks: Vec<Track>,
    f: impl FnOnce(Vec<Track>) -> T + Send + 'static,
) -> Result<T> {
    if tracks.len() >= 1000 {
        tokio::task::spawn_blocking(move || f(tracks))
            .await
            .map_err(|e| LibraryError::Invalid(e.to_string()))
    } else {
        Ok(f(tracks))
    }
}
async fn selection(
    tracks: Vec<Track>,
    from_index: Option<usize>,
    distinct: bool,
) -> Result<PlaybackSelection> {
    transform(tracks, move |tracks| {
        let index = from_index.filter(|i| *i < tracks.len()).unwrap_or(0);
        if !distinct {
            return PlaybackSelection {
                tracks,
                start_index: index,
            };
        }
        let key = tracks.get(index).map(|t| dedupe::key(&t.title, &t.artist));
        let tracks = dedupe::deduplicate_tracks(tracks).items;
        let start_index = key
            .and_then(|key| {
                tracks
                    .iter()
                    .position(|t| dedupe::key(&t.title, &t.artist) == key)
            })
            .unwrap_or(0);
        PlaybackSelection {
            tracks,
            start_index,
        }
    })
    .await
}

fn invalidate_union(c: &ap_store::rusqlite::Connection, id: &str) -> ap_store::Result<()> {
    cache::remove_owner(c, &cache::union_owner_key(id))?;
    c.execute(
        "UPDATE custom_playlists SET updated_at=?2,updated_at_iso=NULL WHERE id=?1",
        ap_store::rusqlite::params![id, ap_store::now_secs()],
    )?;
    Ok(())
}

fn saved_tracks(c: &ap_store::rusqlite::Connection, id: &str) -> ap_store::Result<Vec<Track>> {
    store::list_tracks(c, id)?
        .iter()
        .map(|row| {
            let mut track = row.to_track();
            track.url = c.query_row(
                "SELECT url FROM playlist_tracks WHERE id=?1",
                [row.id],
                |r| r.get(0),
            )?;
            Ok(track)
        })
        .collect()
}

fn clear_timestamp(c: &ap_store::rusqlite::Connection, id: &str) -> ap_store::Result<()> {
    c.execute(
        "UPDATE custom_playlists SET updated_at_iso=NULL WHERE id=?1",
        [id],
    )?;
    Ok(())
}
