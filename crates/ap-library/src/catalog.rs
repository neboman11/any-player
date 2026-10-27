use std::{collections::HashMap, sync::Arc};

use ap_model::{Playlist, SourceType, Track};
use ap_providers::ProviderClient;
use ap_store::{Db, catalog as cache};
use futures_util::{StreamExt, future::join_all, stream};
use tokio::sync::{Mutex, RwLock, broadcast, watch};

use crate::{LibraryError, Result};

const STALE_MS: i64 = 6 * 60 * 60 * 1000;
const SOURCE_ORDER: [SourceType; 3] = [SourceType::Jellyfin, SourceType::Plex, SourceType::Spotify];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRefreshFailure {
    pub source: SourceType,
    pub message: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Tracks,
    Playlists,
    All,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
}

#[derive(Clone)]
pub struct CatalogRepository {
    inner: Arc<Inner>,
}
struct Inner {
    db: Db,
    clients: RwLock<HashMap<SourceType, Arc<dyn ProviderClient>>>,
    refresh: Mutex<()>,
    track_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    playlists: watch::Sender<Vec<Playlist>>,
    changes: broadcast::Sender<String>,
}

impl CatalogRepository {
    /// Clients represent connected providers; replace them after credentials change.
    pub async fn new(db: Db, clients: Vec<Arc<dyn ProviderClient>>) -> Result<Self> {
        let listing = db
            .read(|c| cache::playlists(c, None))
            .await?
            .into_iter()
            .map(|p| p.playlist)
            .collect();
        let (playlists, _) = watch::channel(listing);
        let (changes, _) = broadcast::channel(128);
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                clients: RwLock::new(clients.into_iter().map(|c| (c.source(), c)).collect()),
                refresh: Mutex::new(()),
                track_locks: Mutex::new(HashMap::new()),
                playlists,
                changes,
            }),
        })
    }
    pub async fn set_clients(&self, clients: Vec<Arc<dyn ProviderClient>>) {
        *self.inner.clients.write().await = clients.into_iter().map(|c| (c.source(), c)).collect();
    }
    /// Cache-only observation. No network request is made by subscribing.
    pub fn observe_playlists(&self) -> watch::Receiver<Vec<Playlist>> {
        self.inner.playlists.subscribe()
    }
    /// Owner keys whose cached tracks changed; read them with `cached_tracks`.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.changes.subscribe()
    }
    pub async fn cached_playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self
            .inner
            .db
            .read(|c| cache::playlists(c, None))
            .await?
            .into_iter()
            .map(|p| p.playlist)
            .collect())
    }
    async fn publish_playlists(&self) -> Result<()> {
        self.inner
            .playlists
            .send_replace(self.cached_playlists().await?);
        Ok(())
    }
    pub async fn refresh_playlists(&self) -> Result<Vec<ProviderRefreshFailure>> {
        let _guard = self.inner.refresh.lock().await;
        let clients = self.ordered_clients(SourceType::All).await;
        let outcomes = join_all(clients.into_iter().map(|client| async move {
            let source = client.source();
            let result: Result<()> = async {
                let rows = client.playlists().await?;
                self.inner
                    .db
                    .call(move |c| cache::replace_playlists(c, source, &rows, ap_store::now_ms()))
                    .await?;
                Ok(())
            }
            .await;
            result.err().map(|e| ProviderRefreshFailure {
                source,
                message: e.to_string(),
            })
        }))
        .await;
        self.publish_playlists().await?;
        Ok(outcomes.into_iter().flatten().collect())
    }
    pub async fn cached_tracks(&self, source: SourceType, id: &str) -> Result<Vec<Track>> {
        let key = cache::provider_owner_key(source, &normalize_id(source, id));
        Ok(self
            .inner
            .db
            .read(move |c| cache::tracks(c, &key))
            .await?
            .map_or_else(Vec::new, |c| c.tracks))
    }
    pub async fn tracks(&self, source: SourceType, id: &str, force: bool) -> Result<Vec<Track>> {
        let id = normalize_id(source, id);
        let key = cache::provider_owner_key(source, &id);
        let cached = self.inner.db.read(move |c| cache::tracks(c, &key)).await?;
        if !force && let Some(cached) = cached {
            if ap_store::now_ms().saturating_sub(cached.updated_at_ms) >= STALE_MS {
                let this = self.clone();
                tokio::spawn(async move {
                    // Offline revalidation intentionally retains the previous cache on failure.
                    let _ = this
                        .fetch_tracks(source, id, Some(cached.updated_at_ms))
                        .await;
                });
            }
            return Ok(cached.tracks);
        }
        self.fetch_tracks(source, id, None).await
    }
    async fn fetch_tracks(
        &self,
        source: SourceType,
        id: String,
        stale: Option<i64>,
    ) -> Result<Vec<Track>> {
        let key = cache::provider_owner_key(source, &id);
        let lock = self
            .inner
            .track_locks
            .lock()
            .await
            .entry(key.clone())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        if let Some(timestamp) = stale {
            let read_key = key.clone();
            let current = self
                .inner
                .db
                .read(move |c| cache::tracks(c, &read_key))
                .await?;
            if current.as_ref().map(|c| c.updated_at_ms) != Some(timestamp) {
                return Ok(current.map_or_else(Vec::new, |c| c.tracks));
            }
        }
        let client = self.inner.clients.read().await.get(&source).cloned();
        let Some(client) = client else {
            return self.cached_tracks(source, &id).await;
        };
        let tracks = client.playlist_tracks(&id).await?;
        if tracks.is_empty() {
            let cached = self.cached_tracks(source, &id).await?;
            if !cached.is_empty() {
                self.inner
                    .db
                    .call(move |c| cache::touch_owner(c, &key, ap_store::now_ms()))
                    .await?;
                return Ok(cached);
            }
        }
        let stored = tracks.clone();
        self.inner
            .db
            .call(move |c| {
                cache::replace_provider_tracks(c, source, &id, &stored, ap_store::now_ms())
            })
            .await?;
        let _ = self.inner.changes.send(key);
        self.publish_playlists().await?;
        Ok(tracks)
    }
    pub async fn refresh_all(&self) -> Result<Vec<ProviderRefreshFailure>> {
        let mut failures = self.refresh_playlists().await?;
        let playlists = self.cached_playlists().await?;
        let outcomes: Vec<_> = stream::iter(playlists)
            .map(|p| async move {
                self.tracks(p.source, &p.id, true)
                    .await
                    .err()
                    .map(|e| ProviderRefreshFailure {
                        source: p.source,
                        message: e.to_string(),
                    })
            })
            .buffer_unordered(4)
            .collect()
            .await;
        failures.extend(outcomes.into_iter().flatten());
        Ok(failures)
    }
    async fn ordered_clients(&self, source: SourceType) -> Vec<Arc<dyn ProviderClient>> {
        let clients = self.inner.clients.read().await;
        SOURCE_ORDER
            .into_iter()
            .filter(|s| source == SourceType::All || source == *s)
            .filter_map(|s| clients.get(&s).cloned())
            .collect()
    }
    /// Dropping this future cancels all outstanding searches, without detached tasks.
    pub async fn search(
        &self,
        query: &str,
        source: SourceType,
        kind: SearchType,
    ) -> Result<SearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(SearchResult::default());
        }
        let clients = self.ordered_clients(source).await;
        let outcomes = join_all(clients.into_iter().map(|client| async move {
            let result: Result<SearchResult> = async {
                let tracks = if matches!(kind, SearchType::Tracks | SearchType::All) {
                    client.search_tracks(query).await?
                } else {
                    vec![]
                };
                let playlists = if matches!(kind, SearchType::Playlists | SearchType::All) {
                    client.search_playlists(query).await?
                } else {
                    vec![]
                };
                Ok(SearchResult { tracks, playlists })
            }
            .await;
            result.map_err(|e| provider_search_failure(client.source(), &e.to_string()))
        }))
        .await;
        let mut merged = SearchResult::default();
        let mut successes = 0;
        let mut failures = vec![];
        for result in outcomes {
            match result {
                Ok(mut r) => {
                    successes += 1;
                    merged.tracks.append(&mut r.tracks);
                    merged.playlists.append(&mut r.playlists);
                }
                Err(e) => failures.push(e),
            }
        }
        if successes == 0 && !failures.is_empty() {
            return Err(LibraryError::Search(failures.join("; ")));
        }
        Ok(merged)
    }
    pub async fn clear_cache(&self) -> Result<()> {
        self.inner
            .db
            .call(|c| {
                ap_store::in_tx(c, |c| {
                    for source in SOURCE_ORDER {
                        cache::clear_provider(c, source)?;
                    }
                    Ok(())
                })
            })
            .await?;
        self.publish_playlists().await
    }
    pub async fn is_distinct(&self, source: SourceType, id: &str) -> Result<bool> {
        let id = normalize_id(source, id);
        Ok(self
            .inner
            .db
            .read(move |c| ap_store::playlists::provider_playlist_distinct(c, source, &id))
            .await?)
    }
    pub async fn set_distinct(&self, source: SourceType, id: &str, distinct: bool) -> Result<()> {
        let id = normalize_id(source, id);
        self.inner
            .db
            .call(move |c| {
                ap_store::playlists::set_provider_playlist_distinct(c, source, &id, distinct)
            })
            .await?;
        Ok(())
    }
}

pub(crate) fn normalize_id(source: SourceType, id: &str) -> String {
    ap_model::ids::normalize_playlist_id(source, id).to_owned()
}

fn provider_search_failure(source: SourceType, message: &str) -> String {
    let label = match source {
        SourceType::Jellyfin => "Jellyfin",
        SourceType::Plex => "Plex",
        SourceType::Spotify => "Spotify",
        SourceType::Custom => "Custom",
        SourceType::All => "All",
    };
    let message = if message.trim().is_empty() {
        "Unknown error"
    } else {
        message
    };
    if message
        .get(..label.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(label))
    {
        message.to_owned()
    } else {
        format!("{label}: {message}")
    }
}

#[cfg(test)]
mod tests {
    use super::provider_search_failure;
    use ap_model::SourceType;
    #[test]
    fn android_provider_failure_labels_preserve_existing_prefixes() {
        for (source, message) in [
            (SourceType::Jellyfin, "Jellyfin: HTTP 503"),
            (SourceType::Plex, "PLEX is offline"),
            (SourceType::Spotify, "sPoTiFy token validation failed"),
        ] {
            assert_eq!(provider_search_failure(source, message), message);
        }
        assert_eq!(
            provider_search_failure(SourceType::Jellyfin, "HTTP 503"),
            "Jellyfin: HTTP 503"
        );
        assert_eq!(
            provider_search_failure(SourceType::Spotify, "Plex is offline"),
            "Spotify: Plex is offline"
        );
        for message in ["", "   "] {
            assert_eq!(
                provider_search_failure(SourceType::Spotify, message),
                "Spotify: Unknown error"
            );
        }
    }
}
