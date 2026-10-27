//! Shared Android PlaylistImporter semantics for state files, config files and sync.
//! Connections are supplied by the application's credential store. Spotify access tokens never
//! enter it through transfer. Playback settings are returned, not applied to a live player.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use ap_model::json::JsonElement;
use ap_model::{
    ColumnPreferences, CustomPlaylist, PlaybackStatus, PlaylistTrack, PlaylistType, RepeatMode,
    SourceType, UnionPlaylistSource,
    ids::name_uuid_from_bytes,
    wire::{
        AnyPlayerStateData, AnyPlayerStateEnvelope, CONFIG_EXPORT_VERSION, ConfigCustomPlaylist,
        ConfigFile, ConfigPlaylistEntry, ConfigProviderConfigs, ConfigServer, ConfigTrack,
        ConfigUnionSource, ExportedConnection, SourceAppInfo, config_track_row_id,
        config_union_source_row_id, decode_and_verify, parse_config_playlist_type,
        parse_config_source_type,
    },
};
use ap_store::{
    Db, StoreError, catalog, in_tx, playlists as store,
    rusqlite::{Connection, OptionalExtension, params},
};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use indexmap::IndexMap;

use crate::{LibraryError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    Portable,
    Private,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePolicy {
    ReplaceAll,
    MergeKeepLocal,
    MergePreferImport,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPlaybackSettings {
    pub volume: i32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub playlists_added: usize,
    pub playlists_updated: usize,
    pub tracks_added: usize,
    pub tracks_updated: usize,
    pub union_sources_added: usize,
    pub union_sources_updated: usize,
    pub connections_imported: usize,
    pub connections_skipped: usize,
    pub warnings: Vec<String>,
    pub playback_settings: Option<ImportedPlaybackSettings>,
    /// Non-secret URL changes. Transactional callers apply these only after commit.
    pub connection_updates: Vec<ConnectionUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionUpdate {
    pub source: SourceType,
    pub server_url: String,
}
impl ImportSummary {
    pub fn format(&self, prefix: &str) -> String {
        format!(
            "{prefix} complete. playlists +{}/~{}, tracks +{}/~{}, union +{}/~{}, connections +{}/skip {}, {}",
            self.playlists_added,
            self.playlists_updated,
            self.tracks_added,
            self.tracks_updated,
            self.union_sources_added,
            self.union_sources_updated,
            self.connections_imported,
            self.connections_skipped,
            if self.warnings.is_empty() {
                "no warnings".into()
            } else {
                self.warnings.join(" | ")
            }
        )
    }
}
impl fmt::Display for ImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format("Import"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConnection {
    pub source: SourceType,
    pub server_url: Option<String>,
    pub username: Option<String>,
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub spotify_premium: Option<bool>,
    pub playback_ready: Option<bool>,
}
impl StoredConnection {
    pub fn new(source: SourceType) -> Self {
        Self {
            source,
            server_url: None,
            username: None,
            token: None,
            refresh_token: None,
            token_expires_at: None,
            spotify_premium: None,
            playback_ready: None,
        }
    }
}
/// Application adapter persists non-secrets in settings and tokens in the OS keyring.
#[async_trait]
pub trait ConnectionStore: Send + Sync {
    async fn snapshot(&self) -> Result<IndexMap<SourceType, StoredConnection>>;
    async fn put(&self, connection: StoredConnection) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct PlaylistImportPlan {
    pub data: AnyPlayerStateData,
    pub summary: ImportSummary,
}

/// Pure Android `planPlaylistImport`, including deterministic collision remapping and stable
/// normalization of gapped/colliding positions. Child references must belong to this payload.
pub fn plan_playlist_import(
    incoming: AnyPlayerStateData,
    existing: &AnyPlayerStateData,
    policy: MergePolicy,
    suffix: &str,
    validate_custom_references: bool,
) -> Result<PlaylistImportPlan> {
    let local: HashMap<_, _> = existing
        .custom_playlists
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();
    let mut remap = HashMap::new();
    let mut summary = ImportSummary::default();
    let mut data = incoming;
    let remapped = |id: &str| format!("{id}_{suffix}_{}", name_uuid_from_bytes(id.as_bytes()));
    for p in &mut data.custom_playlists {
        match local.get(p.id.as_str()) {
            None => summary.playlists_added += 1,
            Some(_) if policy != MergePolicy::MergeKeepLocal => summary.playlists_updated += 1,
            Some(old) if old.name != p.name || old.updated_at != p.updated_at => {
                let id = remapped(&p.id);
                remap.insert(p.id.clone(), id.clone());
                p.id = id;
                summary.playlists_added += 1;
            }
            Some(_) => {}
        }
    }
    let old_tracks: HashMap<_, _> = existing
        .playlist_tracks
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();
    let old_sources: HashMap<_, _> = existing
        .union_playlist_sources
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();
    for t in &mut data.playlist_tracks {
        let parent = remap.get(&t.playlist_id).unwrap_or(&t.playlist_id);
        if parent != &t.playlist_id
            || old_tracks
                .get(t.id.as_str())
                .is_some_and(|old| &old.playlist_id != parent)
        {
            t.id = remapped(&format!("{}:{parent}", t.id));
        }
        t.playlist_id = parent.clone();
        if old_tracks.contains_key(t.id.as_str()) {
            summary.tracks_updated += 1;
        } else {
            summary.tracks_added += 1;
        }
    }
    for s in &mut data.union_playlist_sources {
        let parent = remap
            .get(&s.union_playlist_id)
            .unwrap_or(&s.union_playlist_id);
        if parent != &s.union_playlist_id
            || old_sources
                .get(s.id.as_str())
                .is_some_and(|old| &old.union_playlist_id != parent)
        {
            s.id = remapped(&format!("{}:{parent}", s.id));
        }
        s.union_playlist_id = parent.clone();
        if s.source_type == SourceType::Custom
            && let Some(id) = remap.get(&s.source_playlist_id)
        {
            s.source_playlist_id = id.clone();
        }
        if old_sources.contains_key(s.id.as_str()) {
            summary.union_sources_updated += 1;
        } else {
            summary.union_sources_added += 1;
        }
    }
    let ids: HashSet<_> = data
        .custom_playlists
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    for t in &data.playlist_tracks {
        if !ids.contains(t.playlist_id.as_str()) {
            return Err(LibraryError::Invalid(format!(
                "Malformed reference: track {} points to missing playlist {}",
                t.id, t.playlist_id
            )));
        }
    }
    for s in &data.union_playlist_sources {
        if !ids.contains(s.union_playlist_id.as_str())
            || (validate_custom_references
                && s.source_type == SourceType::Custom
                && !ids.contains(s.source_playlist_id.as_str()))
        {
            return Err(LibraryError::Invalid(format!(
                "Malformed union source reference: {}",
                s.id
            )));
        }
    }
    // IndexMap reproduces Kotlin groupBy's first-parent encounter order.
    let mut tracks = IndexMap::<String, Vec<PlaylistTrack>>::new();
    for t in data.playlist_tracks {
        tracks.entry(t.playlist_id.clone()).or_default().push(t);
    }
    data.playlist_tracks = tracks
        .into_values()
        .flat_map(|mut rows| {
            rows.sort_by_key(|t| t.position);
            for (i, t) in rows.iter_mut().enumerate() {
                t.position = i32::try_from(i).unwrap_or(i32::MAX);
            }
            rows
        })
        .collect();
    let mut sources = IndexMap::<String, Vec<UnionPlaylistSource>>::new();
    for s in data.union_playlist_sources {
        sources
            .entry(s.union_playlist_id.clone())
            .or_default()
            .push(s);
    }
    data.union_playlist_sources = sources
        .into_values()
        .flat_map(|mut rows| {
            rows.sort_by_key(|s| s.position);
            for (i, s) in rows.iter_mut().enumerate() {
                s.position = i32::try_from(i).unwrap_or(i32::MAX);
            }
            rows
        })
        .collect();
    Ok(PlaylistImportPlan { data, summary })
}

#[derive(Clone)]
pub struct TransferService {
    db: Db,
    connections: Arc<dyn ConnectionStore>,
}
impl TransferService {
    pub fn new(db: Db, connections: Arc<dyn ConnectionStore>) -> Self {
        Self { db, connections }
    }
    pub async fn snapshot(&self) -> Result<AnyPlayerStateData> {
        Ok(self.db.call(|c| in_tx(c, |c| snapshot(c, true))).await?)
    }
    pub async fn import_data(
        &self,
        data: AnyPlayerStateData,
        policy: MergePolicy,
        dry_run: bool,
        suffix: &str,
        validate_custom_references: bool,
    ) -> Result<ImportSummary> {
        let suffix = suffix.to_owned();
        Ok(self
            .db
            .call(move |c| {
                import_data_conn(
                    c,
                    data,
                    policy,
                    dry_run,
                    &suffix,
                    validate_custom_references,
                )
                .map_err(|e| StoreError::Invalid(e.to_string()))
            })
            .await?)
    }
    pub async fn export_state(
        &self,
        mode: ExportMode,
        passphrase: Option<&str>,
        playback: Option<PlaybackStatus>,
        app_version: &str,
    ) -> Result<String> {
        self.export_state_at(
            mode,
            passphrase,
            playback,
            app_version,
            Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true),
        )
        .await
    }
    pub async fn export_state_at(
        &self,
        mode: ExportMode,
        passphrase: Option<&str>,
        playback: Option<PlaybackStatus>,
        app_version: &str,
        created_at: String,
    ) -> Result<String> {
        if mode == ExportMode::Private && passphrase.is_none() {
            return Err(LibraryError::Invalid(
                "Passphrase is required for private exports".into(),
            ));
        }
        let mut data = self.snapshot().await?;
        for (source, c) in self.connections.snapshot().await? {
            let token = if mode == ExportMode::Private && source != SourceType::Spotify {
                c.token
            } else {
                None
            };
            data.connections.insert(
                source,
                ExportedConnection {
                    source,
                    server_url: c.server_url,
                    username: c.username,
                    api_key: if source == SourceType::Jellyfin {
                        token.clone()
                    } else {
                        None
                    },
                    token,
                    spotify_connected: Some(source == SourceType::Spotify),
                    spotify_premium: c.spotify_premium,
                },
            );
        }
        data.playback_state = playback;
        let plain = AnyPlayerStateEnvelope::new(
            created_at,
            SourceAppInfo {
                platform: "desktop".into(),
                app_version: app_version.into(),
            },
            data,
        )
        .to_json();
        if mode == ExportMode::Private {
            let plain = tokio::task::spawn_blocking({
                let passphrase = passphrase.unwrap_or_default().to_owned();
                move || {
                    ap_model::transfer_crypto::encrypt(
                        &plain,
                        &passphrase,
                        150_000,
                        &mut rand::rng(),
                    )
                }
            })
            .await
            .map_err(|e| LibraryError::Invalid(e.to_string()))?
            .map_err(|e| LibraryError::Invalid(e.to_string()))?;
            Ok(serde_json::to_string(&plain)?)
        } else {
            Ok(plain)
        }
    }
    pub async fn import_state(
        &self,
        raw: &str,
        policy: MergePolicy,
        passphrase: Option<&str>,
        dry_run: bool,
    ) -> Result<ImportSummary> {
        let raw = raw.to_owned();
        let pass = passphrase.map(str::to_owned);
        let envelope =
            tokio::task::spawn_blocking(move || decode_and_verify(&raw, pass.as_deref()))
                .await
                .map_err(|e| LibraryError::Invalid(e.to_string()))??;
        let mut summary = self
            .import_data(envelope.data.clone(), policy, dry_run, "import", true)
            .await?;
        let mut stored = self.connections.snapshot().await?;
        for c in envelope.data.connections.values() {
            let token = if passphrase.is_some() {
                c.token.as_ref().or(c.api_key.as_ref())
            } else {
                None
            };
            if c.source == SourceType::Spotify && token.is_some() {
                summary
                    .warnings
                    .push("Spotify raw access token ignored by policy".into());
            }
            let accepted =
                token.filter(|t| !t.trim().is_empty() && c.source != SourceType::Spotify);
            if c.server_url.as_ref().is_none_or(|s| s.trim().is_empty())
                && accepted.is_none()
                && c.spotify_connected != Some(true)
            {
                summary.connections_skipped += 1;
                continue;
            }
            if !dry_run {
                let mut local = stored
                    .shift_remove(&c.source)
                    .unwrap_or_else(|| StoredConnection::new(c.source));
                if let Some(url) = c.server_url.as_ref().filter(|s| !s.trim().is_empty()) {
                    local.server_url = Some(url.clone());
                }
                if c.username.is_some() {
                    local.username.clone_from(&c.username);
                }
                if let Some(token) = accepted {
                    local.token = Some(token.clone());
                }
                if c.spotify_premium.is_some() {
                    local.spotify_premium = c.spotify_premium;
                }
                local
                    .playback_ready
                    .get_or_insert(c.source != SourceType::Spotify);
                self.connections.put(local.clone()).await?;
                stored.insert(c.source, local);
            }
            summary.connections_imported += 1;
        }
        if !dry_run && policy != MergePolicy::MergeKeepLocal {
            summary.playback_settings =
                envelope
                    .data
                    .playback_state
                    .map(|p| ImportedPlaybackSettings {
                        volume: p.volume,
                        shuffle: p.shuffle,
                        repeat: p.repeat_mode,
                    });
        }
        Ok(summary)
    }
    pub async fn export_playlists_json(&self) -> Result<JsonElement> {
        self.db
            .call(|c| export_playlists_json_conn(c).map_err(|e| StoreError::Invalid(e.to_string())))
            .await
            .map_err(Into::into)
    }
    pub async fn export_provider_config_json(&self) -> Result<JsonElement> {
        let stored = self.connections.snapshot().await?;
        self.db
            .read(move |c| {
                export_provider_config_json_conn(c, &stored)
                    .map_err(|e| StoreError::Invalid(e.to_string()))
            })
            .await
            .map_err(Into::into)
    }
    pub async fn export_config(&self) -> Result<String> {
        let config = ConfigFile {
            export_version: CONFIG_EXPORT_VERSION,
            provider_configs: serde_json::from_str(
                &self.export_provider_config_json().await?.to_string(),
            )?,
            custom_playlists: config_entries(self.snapshot().await?)?,
        };
        Ok(serde_json::to_string(&config)?)
    }
    pub async fn import_config(
        &self,
        raw: &str,
        policy: MergePolicy,
        dry_run: bool,
    ) -> Result<ImportSummary> {
        self.import_config_value(serde_json::from_str(raw)?, policy, dry_run)
            .await
    }
    pub async fn import_json(
        &self,
        value: &JsonElement,
        policy: MergePolicy,
        dry_run: bool,
    ) -> Result<ImportSummary> {
        self.import_config(&value.to_string(), policy, dry_run)
            .await
    }
    async fn import_config_value(
        &self,
        config: ConfigFile,
        policy: MergePolicy,
        dry_run: bool,
    ) -> Result<ImportSummary> {
        let mut summary = self
            .db
            .call(move |c| {
                in_tx(c, |c| {
                    import_config_conn(c, config, policy, dry_run)
                        .map_err(|e| StoreError::Invalid(e.to_string()))
                })
            })
            .await?;
        if !summary.connection_updates.is_empty() {
            let result = self
                .apply_connection_updates(std::mem::take(&mut summary.connection_updates))
                .await;
            if let Err(e) = result {
                summary.warnings.push(format!(
                    "Provider connection import error (playlists were still imported): {e}"
                ));
            }
        }
        Ok(summary)
    }

    /// Applies deferred URL writes after a sync transaction has committed, preserving credentials.
    pub async fn apply_connection_updates(&self, updates: Vec<ConnectionUpdate>) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let mut stored = self.connections.snapshot().await?;
        for update in updates {
            let mut local = stored
                .shift_remove(&update.source)
                .unwrap_or_else(|| StoredConnection::new(update.source));
            local.server_url = Some(update.server_url);
            self.connections.put(local.clone()).await?;
            stored.insert(update.source, local);
        }
        Ok(())
    }
}

fn iso(seconds: i64) -> Result<String> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true))
        .ok_or_else(|| LibraryError::Invalid(format!("Timestamp out of range: {seconds}")))
}
fn epoch(raw: &str) -> Result<i64> {
    DateTime::parse_from_rfc3339(raw)
        .map(|d| d.timestamp())
        .map_err(|e| LibraryError::Invalid(format!("Invalid timestamp '{raw}': {e}")))
}
fn sql_epoch(raw: &str) -> ap_store::Result<i64> {
    epoch(raw).map_err(|e| StoreError::Invalid(e.to_string()))
}
fn sql_iso(seconds: i64) -> ap_store::Result<String> {
    iso(seconds).map_err(|e| StoreError::Invalid(e.to_string()))
}

fn snapshot(c: &Connection, persist_ids: bool) -> ap_store::Result<AnyPlayerStateData> {
    let mut data = AnyPlayerStateData::default();
    let mut track_ids = c
        .prepare("SELECT wire_id FROM playlist_tracks WHERE wire_id IS NOT NULL")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<ap_store::rusqlite::Result<HashSet<_>>>()?;
    let mut source_ids = c
        .prepare("SELECT wire_id FROM union_playlist_sources WHERE wire_id IS NOT NULL")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<ap_store::rusqlite::Result<HashSet<_>>>()?;
    for p in store::list_playlists(c)? {
        let times: (Option<String>, Option<String>) = c.query_row(
            "SELECT created_at_iso,updated_at_iso FROM custom_playlists WHERE id=?1",
            [&p.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let created_at = times
            .0
            .filter(|s| sql_epoch(s).ok() == Some(p.created_at))
            .map_or_else(|| sql_iso(p.created_at), Ok)?;
        let updated_at = times
            .1
            .filter(|s| sql_epoch(s).ok() == Some(p.updated_at))
            .map_or_else(|| sql_iso(p.updated_at), Ok)?;
        let mut occurrences = HashMap::<(SourceType, String), usize>::new();
        for t in store::list_tracks(c, &p.id)? {
            let wire: (Option<String>, Option<String>, Option<String>) = c.query_row(
                "SELECT wire_id,added_at_iso,url FROM playlist_tracks WHERE id=?1",
                [t.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let occurrence = occurrences
                .entry((t.track_source, t.track_id.clone()))
                .or_default();
            let id = resolve_wire_id(wire.0, occurrence, &mut track_ids, |n| {
                config_track_row_id(&p.id, t.track_source, &t.track_id, n)
            });
            if persist_ids {
                c.execute(
                    "UPDATE playlist_tracks SET wire_id=?2 WHERE id=?1 AND wire_id IS NULL",
                    params![t.id, id],
                )?;
            }
            let added_at = wire
                .1
                .filter(|s| sql_epoch(s).ok() == Some(t.added_at))
                .map_or_else(|| sql_iso(t.added_at), Ok)?;
            data.playlist_tracks.push(PlaylistTrack {
                id,
                playlist_id: t.playlist_id,
                track_source: t.track_source,
                track_id: t.track_id,
                position: i32::try_from(t.position).unwrap_or(i32::MAX),
                added_at,
                title: t.title,
                artist: t.artist,
                album: t.album,
                duration_ms: t.duration_ms,
                image_url: t.image_url,
                url: wire.2,
            });
        }
        let mut occurrences = HashMap::<(SourceType, String), usize>::new();
        for s in store::list_union_sources(c, &p.id)? {
            let wire: (Option<String>, Option<String>) = c.query_row(
                "SELECT wire_id,added_at_iso FROM union_playlist_sources WHERE id=?1",
                [s.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let occurrence = occurrences
                .entry((s.source_type, s.source_playlist_id.clone()))
                .or_default();
            let id = resolve_wire_id(wire.0, occurrence, &mut source_ids, |n| {
                config_union_source_row_id(&p.id, s.source_type, &s.source_playlist_id, n)
            });
            if persist_ids {
                c.execute(
                    "UPDATE union_playlist_sources SET wire_id=?2 WHERE id=?1 AND wire_id IS NULL",
                    params![s.id, id],
                )?;
            }
            let added_at = wire
                .1
                .filter(|raw| sql_epoch(raw).ok() == Some(s.added_at))
                .map_or_else(|| sql_iso(s.added_at), Ok)?;
            data.union_playlist_sources.push(UnionPlaylistSource {
                id,
                union_playlist_id: s.union_playlist_id,
                source_type: s.source_type,
                source_playlist_id: s.source_playlist_id,
                position: i32::try_from(s.position).unwrap_or(i32::MAX),
                added_at,
            });
        }
        data.custom_playlists.push(CustomPlaylist {
            id: p.id,
            name: p.name,
            description: p.description,
            image_url: p.image_url,
            created_at,
            updated_at,
            track_count: i32::try_from(p.track_count).unwrap_or(i32::MAX),
            playlist_type: p.playlist_type,
            is_distinct: p.is_distinct,
        });
    }
    // ColumnPreferences' legacy wire map cannot express widths; preserve widths locally.
    if c.query_row("SELECT EXISTS(SELECT 1 FROM column_preferences)", [], |r| {
        r.get::<_, bool>(0)
    })? {
        let layout = store::column_layout(c)?;
        for (position, index) in layout.column_order.iter().enumerate() {
            if let Some(key) = usize::try_from(*index)
                .ok()
                .and_then(|i| layout.columns.get(i))
            {
                data.column_preferences.insert(
                    key.clone(),
                    ColumnPreferences {
                        key: key.clone(),
                        visible: true,
                        position: i32::try_from(position).unwrap_or(i32::MAX),
                    },
                );
            }
        }
    }
    if let Some(raw) = c
        .query_row(
            "SELECT value FROM settings WHERE key='library.column_preferences'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        data.column_preferences = serde_json::from_str(&raw)?;
    }
    Ok(data)
}

fn apply(c: &Connection, data: &AnyPlayerStateData, replace_all: bool) -> ap_store::Result<()> {
    let incoming: HashSet<_> = data
        .custom_playlists
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    if replace_all {
        for p in store::list_playlists(c)? {
            if !incoming.contains(p.id.as_str()) {
                store::delete_playlist(c, &p.id)?;
            }
        }
    }
    for p in &data.custom_playlists {
        let sources: Vec<_> = data
            .union_playlist_sources
            .iter()
            .filter(|s| s.union_playlist_id == p.id)
            .collect();
        let old_sources = store::list_union_sources(c, &p.id)?;
        let keep_cache = p.playlist_type == PlaylistType::Union
            && store::get_playlist(c, &p.id)?
                .is_some_and(|p| p.playlist_type == PlaylistType::Union)
            && old_sources
                .iter()
                .map(|s| (s.source_type, s.source_playlist_id.as_str()))
                .eq(sources
                    .iter()
                    .map(|s| (s.source_type, s.source_playlist_id.as_str())));
        store::upsert_playlist(
            c,
            &store::PlaylistRecord {
                id: p.id.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
                image_url: p.image_url.clone(),
                created_at: sql_epoch(&p.created_at)?,
                updated_at: sql_epoch(&p.updated_at)?,
                track_count: i64::from(p.track_count),
                playlist_type: p.playlist_type,
                is_distinct: p.is_distinct,
            },
        )?;
        c.execute(
            "UPDATE custom_playlists SET created_at_iso=?2,updated_at_iso=?3 WHERE id=?1",
            params![p.id, p.created_at, p.updated_at],
        )?;
        c.execute("DELETE FROM playlist_tracks WHERE playlist_id=?1", [&p.id])?;
        c.execute(
            "DELETE FROM union_playlist_sources WHERE union_playlist_id=?1",
            [&p.id],
        )?;
        if !keep_cache {
            catalog::remove_owner(c, &catalog::union_owner_key(&p.id))?;
        }
    }
    for t in &data.playlist_tracks {
        c.execute("INSERT INTO playlist_tracks (playlist_id,track_source,track_id,position,added_at,title,artist,album,duration_ms,image_url,wire_id,added_at_iso,url) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", params![t.playlist_id,t.track_source.serial_name(),t.track_id,t.position,sql_epoch(&t.added_at)?,t.title,t.artist,t.album,t.duration_ms,t.image_url.as_deref().map(ap_store::strip_plex_token),t.id,t.added_at,t.url.as_deref().map(ap_store::strip_plex_token)])?;
    }
    for s in &data.union_playlist_sources {
        c.execute("INSERT INTO union_playlist_sources (union_playlist_id,source_type,source_playlist_id,position,added_at,wire_id,added_at_iso) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![s.union_playlist_id,s.source_type.serial_name(),s.source_playlist_id,s.position,sql_epoch(&s.added_at)?,s.id,s.added_at])?;
    }
    if !data.column_preferences.is_empty() {
        let mut layout = store::column_layout(c)?;
        let mut columns: Vec<_> = data
            .column_preferences
            .values()
            .filter(|p| p.visible)
            .collect();
        columns.sort_by_key(|p| p.position);
        layout.columns = columns.into_iter().map(|p| p.key.clone()).collect();
        layout.column_order =
            (0..i64::try_from(layout.columns.len()).unwrap_or(i64::MAX)).collect();
        store::set_column_layout(c, &layout)?;
        c.execute("INSERT INTO settings (key,value) VALUES ('library.column_preferences',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&data.column_preferences)?])?;
    }
    Ok(())
}

fn config_entries(data: AnyPlayerStateData) -> Result<Vec<ConfigCustomPlaylist>> {
    let mut track_id: i32 = 0;
    let mut source_id: i32 = 0;
    let mut entries = vec![];
    for p in data.custom_playlists {
        let mut tracks = vec![];
        for t in data
            .playlist_tracks
            .iter()
            .filter(|t| t.playlist_id == p.id)
        {
            track_id = track_id
                .checked_add(1)
                .ok_or_else(|| LibraryError::Invalid("Too many config tracks".into()))?;
            tracks.push(ConfigTrack {
                id: track_id,
                playlist_id: t.playlist_id.clone(),
                track_source: t.track_source.serial_name().into(),
                track_id: t.track_id.clone(),
                position: t.position,
                added_at: epoch(&t.added_at)?,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                duration_ms: t.duration_ms,
                image_url: t.image_url.clone(),
            });
        }
        let mut union_sources = vec![];
        for s in data
            .union_playlist_sources
            .iter()
            .filter(|s| s.union_playlist_id == p.id)
        {
            source_id = source_id
                .checked_add(1)
                .ok_or_else(|| LibraryError::Invalid("Too many config sources".into()))?;
            union_sources.push(ConfigUnionSource {
                id: source_id,
                union_playlist_id: s.union_playlist_id.clone(),
                source_type: s.source_type.serial_name().into(),
                source_playlist_id: s.source_playlist_id.clone(),
                position: s.position,
                added_at: epoch(&s.added_at)?,
            });
        }
        entries.push(ConfigCustomPlaylist {
            playlist: ConfigPlaylistEntry {
                id: p.id,
                name: p.name,
                description: p.description,
                image_url: p.image_url,
                created_at: epoch(&p.created_at)?,
                updated_at: epoch(&p.updated_at)?,
                track_count: p.track_count,
                playlist_type: p.playlist_type.serial_name().into(),
                is_distinct: p.is_distinct,
            },
            tracks,
            union_sources,
        });
    }
    Ok(entries)
}

fn exact_json(value: &impl serde::Serialize) -> Result<JsonElement> {
    JsonElement::parse(&serde_json::to_string(value)?)
        .map_err(|e| LibraryError::Invalid(e.to_string()))
}

// Existing wire ids survive deletion/reorder. Skip occupied occurrence ids when appending a new
// duplicate after earlier rows were removed, rather than violating the unique wire-id index.
fn resolve_wire_id(
    saved: Option<String>,
    occurrence: &mut usize,
    assigned: &mut HashSet<String>,
    derive: impl Fn(usize) -> String,
) -> String {
    if let Some(saved) = saved {
        *occurrence += 1;
        return saved;
    }
    loop {
        let id = derive(*occurrence);
        *occurrence += 1;
        if assigned.insert(id.clone()) {
            return id;
        }
    }
}

/// Runs playlist planning and application on the caller's connection. Savepoints compose with
/// an outer writer transaction, allowing sync compare/import/base recording to commit together.
pub fn import_data_conn(
    c: &Connection,
    data: AnyPlayerStateData,
    policy: MergePolicy,
    dry_run: bool,
    suffix: &str,
    validate_custom_references: bool,
) -> Result<ImportSummary> {
    Ok(in_tx(c, |c| {
        let existing = snapshot(c, !dry_run)?;
        let plan =
            plan_playlist_import(data, &existing, policy, suffix, validate_custom_references)
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
        if !dry_run {
            apply(c, &plan.data, policy == MergePolicy::ReplaceAll)?;
        }
        Ok(plan.summary)
    })?)
}

/// Exact sync playlists payload, evaluated in the caller's transaction.
pub fn export_playlists_json_conn(c: &Connection) -> Result<JsonElement> {
    let data = in_tx(c, |c| snapshot(c, true))?;
    exact_json(&config_entries(data)?)
}

/// Provider URLs live in the application's keyring, not SQLite. Snapshot connections before
/// entering the writer transaction; no keyring I/O occurs here.
pub fn export_provider_config_json_conn(
    _connection: &Connection,
    stored: &IndexMap<SourceType, StoredConnection>,
) -> Result<JsonElement> {
    let server = |source| {
        stored
            .get(&source)
            .and_then(|c| c.server_url.as_ref())
            .filter(|s| !s.trim().is_empty())
            .map(|s| ConfigServer {
                base_url: Some(s.clone()),
            })
    };
    exact_json(&ConfigProviderConfigs {
        spotify: None,
        jellyfin: server(SourceType::Jellyfin),
        plex: server(SourceType::Plex),
    })
}

/// Imports config/sync payloads in the caller's transaction. Apply `connection_updates` only
/// after that transaction commits; dry runs return no side effects.
pub fn import_json_conn(
    c: &Connection,
    file: &JsonElement,
    policy: MergePolicy,
    dry_run: bool,
) -> Result<ImportSummary> {
    let config = serde_json::from_str(&file.to_string())?;
    Ok(in_tx(c, |c| {
        import_config_conn(c, config, policy, dry_run)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })?)
}

fn import_config_conn(
    c: &Connection,
    config: ConfigFile,
    policy: MergePolicy,
    dry_run: bool,
) -> Result<ImportSummary> {
    if config.export_version != CONFIG_EXPORT_VERSION {
        return Err(LibraryError::Invalid(format!(
            "Unsupported config file version: {}",
            config.export_version
        )));
    }
    let mut data = AnyPlayerStateData::default();
    let mut warnings = vec![];
    let mut tracks = HashMap::<(String, SourceType, String), usize>::new();
    let mut sources = HashMap::<(String, SourceType, String), usize>::new();
    for entry in config.custom_playlists {
        let p = entry.playlist;
        let kind = parse_config_playlist_type(&p.playlist_type).unwrap_or_else(|| {
            warnings.push(format!(
                "Unknown playlist_type '{}' for playlist '{}'; defaulted to 'standard'",
                p.playlist_type, p.id
            ));
            PlaylistType::Standard
        });
        data.custom_playlists.push(CustomPlaylist {
            id: p.id,
            name: p.name,
            description: p.description,
            image_url: p.image_url,
            created_at: iso(p.created_at)?,
            updated_at: iso(p.updated_at)?,
            track_count: p.track_count,
            playlist_type: kind,
            is_distinct: p.is_distinct,
        });
        for t in entry.tracks {
            let Some(source) = parse_config_source_type(&t.track_source) else {
                warnings.push(format!(
                    "Unknown track_source '{}' for track '{}'; skipped",
                    t.track_source, t.track_id
                ));
                continue;
            };
            let occurrence = tracks
                .entry((t.playlist_id.clone(), source, t.track_id.clone()))
                .or_default();
            let id = config_track_row_id(&t.playlist_id, source, &t.track_id, *occurrence);
            *occurrence += 1;
            data.playlist_tracks.push(PlaylistTrack {
                id,
                playlist_id: t.playlist_id,
                track_source: source,
                track_id: t.track_id,
                position: t.position,
                added_at: iso(t.added_at)?,
                title: t.title,
                artist: t.artist,
                album: t.album,
                duration_ms: t.duration_ms,
                image_url: t.image_url,
                url: None,
            });
        }
        for s in entry.union_sources {
            let Some(source) = parse_config_source_type(&s.source_type) else {
                warnings.push(format!(
                    "Unknown source_type '{}' for union source '{}'; skipped",
                    s.source_type, s.source_playlist_id
                ));
                continue;
            };
            let occurrence = sources
                .entry((
                    s.union_playlist_id.clone(),
                    source,
                    s.source_playlist_id.clone(),
                ))
                .or_default();
            let id = config_union_source_row_id(
                &s.union_playlist_id,
                source,
                &s.source_playlist_id,
                *occurrence,
            );
            *occurrence += 1;
            data.union_playlist_sources.push(UnionPlaylistSource {
                id,
                union_playlist_id: s.union_playlist_id,
                source_type: source,
                source_playlist_id: s.source_playlist_id,
                position: s.position,
                added_at: iso(s.added_at)?,
            });
        }
    }
    let mut summary = import_data_conn(c, data, policy, dry_run, "cfg", false)?;
    if config
        .provider_configs
        .spotify
        .is_some_and(|s| s.client_id.is_some() || s.redirect_uri.is_some())
    {
        warnings.push(
            "Spotify client_id/redirect_uri in config are app build settings — skipped".into(),
        );
    }
    for (source, server) in [
        (SourceType::Jellyfin, config.provider_configs.jellyfin),
        (SourceType::Plex, config.provider_configs.plex),
    ] {
        if let Some(server_url) = server
            .and_then(|s| s.base_url)
            .filter(|s| !s.trim().is_empty())
        {
            summary.connections_imported += 1;
            if !dry_run {
                summary
                    .connection_updates
                    .push(ConnectionUpdate { source, server_url });
            }
        }
    }
    summary.warnings = warnings;
    Ok(summary)
}
