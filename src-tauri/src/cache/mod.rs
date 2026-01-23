use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const PLAYLISTS_CACHE_FILE: &str = "playlists_cache.json";
const CUSTOM_PLAYLISTS_CACHE_FILE: &str = "custom_playlists_cache.json";
const CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX: &str = "custom_playlist_tracks_";
const UNION_PLAYLIST_TRACKS_CACHE_PREFIX: &str = "union_playlist_tracks_";

/// Get the XDG cache directory for the application
fn get_cache_dir() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .context("Failed to get cache directory")?
        .join("any-player");

    // Create directory if it doesn't exist
    fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;

    Ok(cache_dir)
}

/// Write data to a cache file
pub fn write_cache<T: Serialize>(filename: &str, data: &T) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join(filename);

    let json = serde_json::to_string(data).context("Failed to serialize cache data")?;

    fs::write(&cache_file, json)
        .with_context(|| format!("Failed to write cache file: {}", cache_file.display()))?;

    tracing::debug!("Wrote cache to {}", cache_file.display());
    Ok(())
}

/// Read data from a cache file
pub fn read_cache<T: for<'de> Deserialize<'de>>(filename: &str) -> Result<Option<T>> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join(filename);

    if !cache_file.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&cache_file)
        .with_context(|| format!("Failed to read cache file: {}", cache_file.display()))?;

    let data: T = serde_json::from_str(&json).context("Failed to deserialize cache data")?;

    tracing::debug!("Read cache from {}", cache_file.display());
    Ok(Some(data))
}

/// Delete a cache file
pub fn clear_cache(filename: &str) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join(filename);

    if cache_file.exists() {
        fs::remove_file(&cache_file)
            .with_context(|| format!("Failed to remove cache file: {}", cache_file.display()))?;
        tracing::debug!("Cleared cache file {}", cache_file.display());
    }

    Ok(())
}

/// Write playlists to cache
pub fn write_playlists_cache(data: &str) -> Result<()> {
    write_cache(PLAYLISTS_CACHE_FILE, &data)
}

/// Read playlists from cache
pub fn read_playlists_cache() -> Result<Option<String>> {
    read_cache(PLAYLISTS_CACHE_FILE)
}

/// Clear playlists cache
pub fn clear_playlists_cache() -> Result<()> {
    clear_cache(PLAYLISTS_CACHE_FILE)
}

/// Write custom playlists to cache
pub fn write_custom_playlists_cache(data: &str) -> Result<()> {
    write_cache(CUSTOM_PLAYLISTS_CACHE_FILE, &data)
}

/// Read custom playlists from cache
pub fn read_custom_playlists_cache() -> Result<Option<String>> {
    read_cache(CUSTOM_PLAYLISTS_CACHE_FILE)
}

/// Clear custom playlists cache
pub fn clear_custom_playlists_cache() -> Result<()> {
    clear_cache(CUSTOM_PLAYLISTS_CACHE_FILE)
}

/// Write custom playlist tracks to cache
pub fn write_custom_playlist_tracks_cache(playlist_id: &str, data: &str) -> Result<()> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    write_cache(&filename, &data)
}

/// Read custom playlist tracks from cache
pub fn read_custom_playlist_tracks_cache(playlist_id: &str) -> Result<Option<String>> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    read_cache(&filename)
}

/// Clear custom playlist tracks cache
pub fn clear_custom_playlist_tracks_cache(playlist_id: &str) -> Result<()> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    clear_cache(&filename)
}

/// Write union playlist tracks to cache
pub fn write_union_playlist_tracks_cache(playlist_id: &str, data: &str) -> Result<()> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    write_cache(&filename, &data)
}

/// Read union playlist tracks from cache
pub fn read_union_playlist_tracks_cache(playlist_id: &str) -> Result<Option<String>> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    read_cache(&filename)
}

/// Clear union playlist tracks cache
pub fn clear_union_playlist_tracks_cache(playlist_id: &str) -> Result<()> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    clear_cache(&filename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestData {
        value: String,
        count: i32,
    }

    #[test]
    fn test_write_and_read_cache() {
        let test_file = "test_cache.json";
        let test_data = TestData {
            value: "test".to_string(),
            count: 42,
        };

        // Write cache
        write_cache(test_file, &test_data).unwrap();

        // Read cache
        let read_data: Option<TestData> = read_cache(test_file).unwrap();
        assert!(read_data.is_some());
        assert_eq!(read_data.unwrap(), test_data);

        // Clean up
        clear_cache(test_file).unwrap();
    }

    #[test]
    fn test_clear_cache() {
        let test_file = "test_clear.json";
        let test_data = TestData {
            value: "test".to_string(),
            count: 42,
        };

        // Write and then clear
        write_cache(test_file, &test_data).unwrap();
        clear_cache(test_file).unwrap();

        // Verify it's gone
        let read_data: Option<TestData> = read_cache(test_file).unwrap();
        assert!(read_data.is_none());
    }
}
