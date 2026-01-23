use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::models::{Source, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPlaylist {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub track_count: i64,
    pub playlist_type: String, // "standard" or "union"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnionPlaylistSource {
    pub id: i64,
    pub union_playlist_id: String,
    pub source_type: String, // "spotify", "jellyfin", "custom"
    pub source_playlist_id: String,
    pub position: i64,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTrack {
    pub id: i64,
    pub playlist_id: String,
    pub track_source: String,
    pub track_id: String,
    pub position: i64,
    pub added_at: i64,
    // Cached metadata
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub image_url: Option<String>,
}

impl PlaylistTrack {
    pub fn to_track(&self) -> Track {
        let source = match self.track_source.as_str() {
            "spotify" => Source::Spotify,
            "jellyfin" => Source::Jellyfin,
            "custom" => Source::Custom,
            _ => Source::Spotify, // Default fallback
        };

        Track {
            id: self.track_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone().unwrap_or_default(),
            duration_ms: self.duration_ms.unwrap_or(0) as u64,
            image_url: self.image_url.clone(),
            source,
            url: None,
            auth_headers: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnPreferences {
    pub columns: Vec<String>,
    pub column_order: Vec<usize>,
    pub column_widths: std::collections::HashMap<String, i64>,
}

impl Default for ColumnPreferences {
    fn default() -> Self {
        Self {
            columns: vec![
                "title".to_string(),
                "artist".to_string(),
                "album".to_string(),
                "duration".to_string(),
                "source".to_string(),
            ],
            column_order: vec![0, 1, 2, 3, 4],
            column_widths: std::collections::HashMap::from([
                ("title".to_string(), 300),
                ("artist".to_string(), 200),
                ("album".to_string(), 200),
                ("duration".to_string(), 100),
                ("source".to_string(), 100),
            ]),
        }
    }
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path).context("Failed to open database")?;
        let db = Database { conn };
        db.initialize_schema()?;
        Ok(db)
    }

    /// Initialize the database schema.
    ///
    /// NOTE: This uses string concatenation for SQL, which is generally safe here
    /// since there is no user input involved. For production applications with
    /// complex schema evolution, consider using a migration tool like `refinery`
    /// or `sqlx-migrate` to track and apply schema changes in a versioned manner.
    fn initialize_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS custom_playlists (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                image_url TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                track_count INTEGER DEFAULT 0,
                playlist_type TEXT DEFAULT 'standard'
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                playlist_id TEXT NOT NULL,
                track_source TEXT NOT NULL,
                track_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                added_at INTEGER NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT,
                duration_ms INTEGER,
                image_url TEXT,
                FOREIGN KEY (playlist_id) REFERENCES custom_playlists(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS union_playlist_sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                union_playlist_id TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source_playlist_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                added_at INTEGER NOT NULL,
                FOREIGN KEY (union_playlist_id) REFERENCES custom_playlists(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS column_preferences (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                columns TEXT NOT NULL,
                column_order TEXT NOT NULL,
                column_widths TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist_id 
                ON playlist_tracks(playlist_id);
            CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position 
                ON playlist_tracks(playlist_id, position);
            CREATE INDEX IF NOT EXISTS idx_union_playlist_sources_union_id
                ON union_playlist_sources(union_playlist_id);
            CREATE INDEX IF NOT EXISTS idx_union_playlist_sources_position
                ON union_playlist_sources(union_playlist_id, position);
            "#,
        )?;

        // Migration: Add playlist_type column if it doesn't exist (for existing databases)
        let has_playlist_type: bool = self.conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('custom_playlists') WHERE name='playlist_type'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0) > 0;

        if !has_playlist_type {
            self.conn.execute(
                "ALTER TABLE custom_playlists ADD COLUMN playlist_type TEXT DEFAULT 'standard'",
                [],
            )?;
        }

        Ok(())
    }

    // Custom Playlist CRUD Operations

    pub fn create_playlist(
        &self,
        name: String,
        description: Option<String>,
        image_url: Option<String>,
    ) -> Result<CustomPlaylist> {
        self.create_playlist_with_type(name, description, image_url, "standard".to_string())
    }

    pub fn create_playlist_with_type(
        &self,
        name: String,
        description: Option<String>,
        image_url: Option<String>,
        playlist_type: String,
    ) -> Result<CustomPlaylist> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        self.conn.execute(
            "INSERT INTO custom_playlists (id, name, description, image_url, created_at, updated_at, track_count, playlist_type) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![id, name, description, image_url, now, now, playlist_type],
        )?;

        Ok(CustomPlaylist {
            id,
            name,
            description,
            image_url,
            created_at: now,
            updated_at: now,
            track_count: 0,
            playlist_type,
        })
    }

    pub fn get_all_playlists(&self) -> Result<Vec<CustomPlaylist>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, image_url, created_at, updated_at, track_count, playlist_type 
             FROM custom_playlists 
             ORDER BY updated_at DESC",
        )?;

        let playlists = stmt
            .query_map([], |row| {
                Ok(CustomPlaylist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    image_url: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    track_count: row.get(6)?,
                    playlist_type: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(playlists)
    }

    pub fn get_playlist(&self, playlist_id: &str) -> Result<Option<CustomPlaylist>> {
        let playlist = self
            .conn
            .query_row(
                "SELECT id, name, description, image_url, created_at, updated_at, track_count, playlist_type 
                 FROM custom_playlists 
                 WHERE id = ?1",
                params![playlist_id],
                |row| {
                    Ok(CustomPlaylist {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        image_url: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        track_count: row.get(6)?,
                        playlist_type: row.get(7)?,
                    })
                },
            )
            .optional()?;

        Ok(playlist)
    }

    pub fn update_playlist(
        &self,
        playlist_id: &str,
        name: Option<String>,
        description: Option<String>,
        image_url: Option<String>,
    ) -> Result<()> {
        // Return early if nothing to update
        if name.is_none() && description.is_none() && image_url.is_none() {
            return Ok(());
        }

        let now = Utc::now().timestamp();

        // Build the SET clause dynamically with only the fields that are provided
        let mut set_clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(n) = name {
            set_clauses.push("name = ?");
            params.push(Box::new(n));
        }
        if let Some(d) = description {
            set_clauses.push("description = ?");
            params.push(Box::new(d));
        }
        if let Some(img) = image_url {
            set_clauses.push("image_url = ?");
            params.push(Box::new(img));
        }

        // Always update the updated_at timestamp
        set_clauses.push("updated_at = ?");
        params.push(Box::new(now));

        // Build the complete query
        let query = format!(
            "UPDATE custom_playlists SET {} WHERE id = ?",
            set_clauses.join(", ")
        );
        params.push(Box::new(playlist_id.to_string()));

        // Execute the query with the dynamically built parameters
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        self.conn.execute(&query, params_refs.as_slice())?;

        Ok(())
    }

    pub fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM custom_playlists WHERE id = ?1",
            params![playlist_id],
        )?;
        Ok(())
    }

    // Track Operations

    pub fn add_track_to_playlist(&self, playlist_id: &str, track: &Track) -> Result<PlaylistTrack> {
        let now = Utc::now().timestamp();

        // Get current max position
        let position: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |row| row.get(0),
            )
            .unwrap_or(-1)
            + 1;

        let source_str = match track.source {
            Source::Spotify => "spotify",
            Source::Jellyfin => "jellyfin",
            Source::Custom => "custom",
        };

        self.conn.execute(
            "INSERT INTO playlist_tracks 
             (playlist_id, track_source, track_id, position, added_at, title, artist, album, duration_ms, image_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                playlist_id,
                source_str,
                track.id,
                position,
                now,
                track.title,
                track.artist,
                track.album,
                track.duration_ms as i64,
                track.image_url
            ],
        )?;

        let track_id = self.conn.last_insert_rowid();

        // Update track count
        self.conn.execute(
            "UPDATE custom_playlists SET track_count = track_count + 1, updated_at = ?1 WHERE id = ?2",
            params![now, playlist_id],
        )?;

        Ok(PlaylistTrack {
            id: track_id,
            playlist_id: playlist_id.to_string(),
            track_source: source_str.to_string(),
            track_id: track.id.clone(),
            position,
            added_at: now,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: Some(track.album.clone()),
            duration_ms: Some(track.duration_ms as i64),
            image_url: track.image_url.clone(),
        })
    }

    pub fn get_playlist_tracks(&self, playlist_id: &str) -> Result<Vec<PlaylistTrack>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, playlist_id, track_source, track_id, position, added_at, 
                    title, artist, album, duration_ms, image_url
             FROM playlist_tracks 
             WHERE playlist_id = ?1 
             ORDER BY position ASC",
        )?;

        let tracks = stmt
            .query_map(params![playlist_id], |row| {
                Ok(PlaylistTrack {
                    id: row.get(0)?,
                    playlist_id: row.get(1)?,
                    track_source: row.get(2)?,
                    track_id: row.get(3)?,
                    position: row.get(4)?,
                    added_at: row.get(5)?,
                    title: row.get(6)?,
                    artist: row.get(7)?,
                    album: row.get(8)?,
                    duration_ms: row.get(9)?,
                    image_url: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(tracks)
    }

    pub fn remove_track_from_playlist(&self, track_id: i64) -> Result<()> {
        let now = Utc::now().timestamp();

        // Get playlist_id and position before deleting
        let (playlist_id, position): (String, i64) = self.conn.query_row(
            "SELECT playlist_id, position FROM playlist_tracks WHERE id = ?1",
            params![track_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Delete the track
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE id = ?1",
            params![track_id],
        )?;

        // Reorder remaining tracks
        self.conn.execute(
            "UPDATE playlist_tracks SET position = position - 1 
             WHERE playlist_id = ?1 AND position > ?2",
            params![playlist_id, position],
        )?;

        // Update track count
        self.conn.execute(
            "UPDATE custom_playlists SET track_count = track_count - 1, updated_at = ?1 WHERE id = ?2",
            params![now, playlist_id],
        )?;

        Ok(())
    }

    pub fn reorder_tracks(
        &self,
        playlist_id: &str,
        track_id: i64,
        new_position: i64,
    ) -> Result<()> {
        let now = Utc::now().timestamp();

        // Get current position
        let old_position: i64 = self.conn.query_row(
            "SELECT position FROM playlist_tracks WHERE id = ?1",
            params![track_id],
            |row| row.get(0),
        )?;

        if old_position == new_position {
            return Ok(());
        }

        if old_position < new_position {
            // Moving down: shift tracks between old and new position up
            self.conn.execute(
                "UPDATE playlist_tracks SET position = position - 1 
                 WHERE playlist_id = ?1 AND position > ?2 AND position <= ?3",
                params![playlist_id, old_position, new_position],
            )?;
        } else {
            // Moving up: shift tracks between new and old position down
            self.conn.execute(
                "UPDATE playlist_tracks SET position = position + 1 
                 WHERE playlist_id = ?1 AND position >= ?2 AND position < ?3",
                params![playlist_id, new_position, old_position],
            )?;
        }

        // Update the track's position
        self.conn.execute(
            "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
            params![new_position, track_id],
        )?;

        // Update playlist timestamp
        self.conn.execute(
            "UPDATE custom_playlists SET updated_at = ?1 WHERE id = ?2",
            params![now, playlist_id],
        )?;

        Ok(())
    }

    // Union Playlist Operations

    pub fn add_source_to_union_playlist(
        &self,
        union_playlist_id: &str,
        source_type: &str,
        source_playlist_id: &str,
    ) -> Result<UnionPlaylistSource> {
        let now = Utc::now().timestamp();

        // Get current max position
        let position: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM union_playlist_sources WHERE union_playlist_id = ?1",
                params![union_playlist_id],
                |row| row.get(0),
            )
            .unwrap_or(-1)
            + 1;

        self.conn.execute(
            "INSERT INTO union_playlist_sources (union_playlist_id, source_type, source_playlist_id, position, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![union_playlist_id, source_type, source_playlist_id, position, now],
        )?;

        let id = self.conn.last_insert_rowid();

        // Update playlist timestamp
        self.conn.execute(
            "UPDATE custom_playlists SET updated_at = ?1 WHERE id = ?2",
            params![now, union_playlist_id],
        )?;

        Ok(UnionPlaylistSource {
            id,
            union_playlist_id: union_playlist_id.to_string(),
            source_type: source_type.to_string(),
            source_playlist_id: source_playlist_id.to_string(),
            position,
            added_at: now,
        })
    }

    pub fn get_union_playlist_sources(
        &self,
        union_playlist_id: &str,
    ) -> Result<Vec<UnionPlaylistSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, union_playlist_id, source_type, source_playlist_id, position, added_at
             FROM union_playlist_sources 
             WHERE union_playlist_id = ?1 
             ORDER BY position ASC",
        )?;

        let sources = stmt
            .query_map(params![union_playlist_id], |row| {
                Ok(UnionPlaylistSource {
                    id: row.get(0)?,
                    union_playlist_id: row.get(1)?,
                    source_type: row.get(2)?,
                    source_playlist_id: row.get(3)?,
                    position: row.get(4)?,
                    added_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sources)
    }

    pub fn remove_source_from_union_playlist(&self, source_id: i64) -> Result<()> {
        let now = Utc::now().timestamp();

        // Get union_playlist_id and position before deleting
        let (union_playlist_id, position): (String, i64) = self.conn.query_row(
            "SELECT union_playlist_id, position FROM union_playlist_sources WHERE id = ?1",
            params![source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Delete the source
        self.conn.execute(
            "DELETE FROM union_playlist_sources WHERE id = ?1",
            params![source_id],
        )?;

        // Reorder remaining sources
        self.conn.execute(
            "UPDATE union_playlist_sources SET position = position - 1 
             WHERE union_playlist_id = ?1 AND position > ?2",
            params![union_playlist_id, position],
        )?;

        // Update playlist timestamp
        self.conn.execute(
            "UPDATE custom_playlists SET updated_at = ?1 WHERE id = ?2",
            params![now, union_playlist_id],
        )?;

        Ok(())
    }

    pub fn reorder_union_sources(
        &self,
        union_playlist_id: &str,
        source_id: i64,
        new_position: i64,
    ) -> Result<()> {
        let now = Utc::now().timestamp();

        // Get current position
        let old_position: i64 = self.conn.query_row(
            "SELECT position FROM union_playlist_sources WHERE id = ?1",
            params![source_id],
            |row| row.get(0),
        )?;

        if old_position == new_position {
            return Ok(());
        }

        if old_position < new_position {
            // Moving down: shift sources between old and new position up
            self.conn.execute(
                "UPDATE union_playlist_sources SET position = position - 1 
                 WHERE union_playlist_id = ?1 AND position > ?2 AND position <= ?3",
                params![union_playlist_id, old_position, new_position],
            )?;
        } else {
            // Moving up: shift sources between new and old position down
            self.conn.execute(
                "UPDATE union_playlist_sources SET position = position + 1 
                 WHERE union_playlist_id = ?1 AND position >= ?2 AND position < ?3",
                params![union_playlist_id, new_position, old_position],
            )?;
        }

        // Update the source's position
        self.conn.execute(
            "UPDATE union_playlist_sources SET position = ?1 WHERE id = ?2",
            params![new_position, source_id],
        )?;

        // Update playlist timestamp
        self.conn.execute(
            "UPDATE custom_playlists SET updated_at = ?1 WHERE id = ?2",
            params![now, union_playlist_id],
        )?;

        Ok(())
    }

    // Column Preferences

    pub fn get_column_preferences(&self) -> Result<ColumnPreferences> {
        let result = self
            .conn
            .query_row(
                "SELECT columns, column_order, column_widths FROM column_preferences WHERE id = 1",
                [],
                |row| {
                    let columns_json: String = row.get(0)?;
                    let order_json: String = row.get(1)?;
                    let widths_json: Option<String> = row.get(2)?;

                    Ok((columns_json, order_json, widths_json))
                },
            )
            .optional()?;

        match result {
            Some((columns_json, order_json, widths_json)) => {
                let columns: Vec<String> = serde_json::from_str(&columns_json)?;
                let column_order: Vec<usize> = serde_json::from_str(&order_json)?;
                let column_widths: std::collections::HashMap<String, i64> = widths_json
                    .map(|json| serde_json::from_str(&json).unwrap_or_default())
                    .unwrap_or_default();

                Ok(ColumnPreferences {
                    columns,
                    column_order,
                    column_widths,
                })
            }
            None => Ok(ColumnPreferences::default()),
        }
    }

    pub fn save_column_preferences(&self, prefs: &ColumnPreferences) -> Result<()> {
        let columns_json = serde_json::to_string(&prefs.columns)?;
        let order_json = serde_json::to_string(&prefs.column_order)?;
        let widths_json = serde_json::to_string(&prefs.column_widths)?;

        self.conn.execute(
            "INSERT OR REPLACE INTO column_preferences (id, columns, column_order, column_widths)
             VALUES (1, ?1, ?2, ?3)",
            params![columns_json, order_json, widths_json],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Database {
        Database::new(":memory:".into()).unwrap()
    }

    #[test]
    fn test_create_and_get_playlist() {
        let db = create_test_db();

        let playlist = db
            .create_playlist(
                "Test Playlist".to_string(),
                Some("Description".to_string()),
                None,
            )
            .unwrap();

        assert_eq!(playlist.name, "Test Playlist");
        assert_eq!(playlist.track_count, 0);

        let retrieved = db.get_playlist(&playlist.id).unwrap().unwrap();
        assert_eq!(retrieved.name, "Test Playlist");
    }

    #[test]
    fn test_add_and_get_tracks() {
        let db = create_test_db();

        let playlist = db.create_playlist("Test".to_string(), None, None).unwrap();

        let track = Track {
            id: "track1".to_string(),
            title: "Song 1".to_string(),
            artist: "Artist 1".to_string(),
            album: "Album 1".to_string(),
            duration_ms: 180000,
            image_url: None,
            source: Source::Spotify,
            url: None,
            auth_headers: None,
        };

        db.add_track_to_playlist(&playlist.id, &track).unwrap();

        let tracks = db.get_playlist_tracks(&playlist.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Song 1");
        assert_eq!(tracks[0].position, 0);
    }

    #[test]
    fn test_reorder_tracks() {
        let db = create_test_db();

        let playlist = db.create_playlist("Test".to_string(), None, None).unwrap();

        // Add 3 tracks
        for i in 0..3 {
            let track = Track {
                id: format!("track{}", i),
                title: format!("Song {}", i),
                artist: "Artist".to_string(),
                album: "Album".to_string(),
                duration_ms: 180000,
                image_url: None,
                source: Source::Spotify,
                url: None,
                auth_headers: None,
            };
            db.add_track_to_playlist(&playlist.id, &track).unwrap();
        }

        let tracks = db.get_playlist_tracks(&playlist.id).unwrap();
        let first_track_id = tracks[0].id;

        // Move first track to position 2
        db.reorder_tracks(&playlist.id, first_track_id, 2).unwrap();

        let reordered = db.get_playlist_tracks(&playlist.id).unwrap();
        assert_eq!(reordered[2].title, "Song 0");
        assert_eq!(reordered[0].title, "Song 1");
    }
}
