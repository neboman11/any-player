use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

const PLAYLISTS_CACHE_FILE: &str = "playlists_cache.json";
const CUSTOM_PLAYLISTS_CACHE_FILE: &str = "custom_playlists_cache.json";
const CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX: &str = "custom_playlist_tracks_";
const UNION_PLAYLIST_TRACKS_CACHE_PREFIX: &str = "union_playlist_tracks_";
const PROVIDER_PLAYLIST_CACHE_PREFIX: &str = "provider_playlist_";

/// Get the XDG cache directory for the application
async fn get_cache_dir() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .context("Failed to get cache directory")?
        .join("any-player");

    // Create directory if it doesn't exist
    fs::create_dir_all(&cache_dir)
        .await
        .context("Failed to create cache directory")?;

    Ok(cache_dir)
}

/// Write data to a cache file
pub async fn write_cache<T: Serialize>(filename: &str, data: &T) -> Result<()> {
    let cache_dir = get_cache_dir().await?;
    let cache_file = cache_dir.join(filename);

    let json = serde_json::to_string(data).context("Failed to serialize cache data")?;

    fs::write(&cache_file, json)
        .await
        .with_context(|| format!("Failed to write cache file: {}", cache_file.display()))?;

    tracing::debug!("Wrote cache to {}", cache_file.display());
    Ok(())
}

/// Read data from a cache file
pub async fn read_cache<T: for<'de> Deserialize<'de>>(filename: &str) -> Result<Option<T>> {
    let cache_dir = get_cache_dir().await?;
    let cache_file = cache_dir.join(filename);

    match fs::metadata(&cache_file).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("Failed to stat cache file: {}", cache_file.display()))
        }
    }

    let json = fs::read_to_string(&cache_file)
        .await
        .with_context(|| format!("Failed to read cache file: {}", cache_file.display()))?;

    let data: T = serde_json::from_str(&json).context("Failed to deserialize cache data")?;

    tracing::debug!("Read cache from {}", cache_file.display());
    Ok(Some(data))
}

/// Delete a cache file
pub async fn clear_cache(filename: &str) -> Result<()> {
    let cache_dir = get_cache_dir().await?;
    let cache_file = cache_dir.join(filename);

    match fs::remove_file(&cache_file).await {
        Ok(_) => tracing::debug!("Cleared cache file {}", cache_file.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("Failed to remove cache file: {}", cache_file.display())
            })
        }
    }

    Ok(())
}

/// Write playlists to cache
pub async fn write_playlists_cache(data: &str) -> Result<()> {
    write_cache(PLAYLISTS_CACHE_FILE, &data).await
}

/// Read playlists from cache
pub async fn read_playlists_cache() -> Result<Option<String>> {
    read_cache(PLAYLISTS_CACHE_FILE).await
}

/// Clear playlists cache
pub async fn clear_playlists_cache() -> Result<()> {
    clear_cache(PLAYLISTS_CACHE_FILE).await
}

/// Write custom playlists to cache
pub async fn write_custom_playlists_cache(data: &str) -> Result<()> {
    write_cache(CUSTOM_PLAYLISTS_CACHE_FILE, &data).await
}

/// Read custom playlists from cache
pub async fn read_custom_playlists_cache() -> Result<Option<String>> {
    read_cache(CUSTOM_PLAYLISTS_CACHE_FILE).await
}

/// Clear custom playlists cache
pub async fn clear_custom_playlists_cache() -> Result<()> {
    clear_cache(CUSTOM_PLAYLISTS_CACHE_FILE).await
}

/// Write custom playlist tracks to cache
pub async fn write_custom_playlist_tracks_cache(playlist_id: &str, data: &str) -> Result<()> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    write_cache(&filename, &data).await
}

/// Read custom playlist tracks from cache
pub async fn read_custom_playlist_tracks_cache(playlist_id: &str) -> Result<Option<String>> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    read_cache(&filename).await
}

/// Clear custom playlist tracks cache
pub async fn clear_custom_playlist_tracks_cache(playlist_id: &str) -> Result<()> {
    let filename = format!(
        "{}{}.json",
        CUSTOM_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id
    );
    clear_cache(&filename).await
}

/// Write union playlist tracks to cache
pub async fn write_union_playlist_tracks_cache(playlist_id: &str, data: &str) -> Result<()> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    write_cache(&filename, &data).await
}

/// Read union playlist tracks from cache
pub async fn read_union_playlist_tracks_cache(playlist_id: &str) -> Result<Option<String>> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    read_cache(&filename).await
}

/// Clear union playlist tracks cache
pub async fn clear_union_playlist_tracks_cache(playlist_id: &str) -> Result<()> {
    let filename = format!("{}{}.json", UNION_PLAYLIST_TRACKS_CACHE_PREFIX, playlist_id);
    clear_cache(&filename).await
}

/// Write provider playlist details to cache
pub async fn write_provider_playlist_cache(
    source: &str,
    playlist_id: &str,
    data: &str,
) -> Result<()> {
    let filename = format!(
        "{}{}_{}.json",
        PROVIDER_PLAYLIST_CACHE_PREFIX, source, playlist_id
    );
    write_cache(&filename, &data).await
}

/// Read provider playlist details from cache
pub async fn read_provider_playlist_cache(
    source: &str,
    playlist_id: &str,
) -> Result<Option<String>> {
    let filename = format!(
        "{}{}_{}.json",
        PROVIDER_PLAYLIST_CACHE_PREFIX, source, playlist_id
    );
    read_cache(&filename).await
}

/// Clear provider playlist details cache
pub async fn clear_provider_playlist_cache(source: &str, playlist_id: &str) -> Result<()> {
    let filename = format!(
        "{}{}_{}.json",
        PROVIDER_PLAYLIST_CACHE_PREFIX, source, playlist_id
    );
    clear_cache(&filename).await
}

/// Clear all provider playlist details caches
pub async fn clear_provider_playlists_cache() -> Result<usize> {
    let cache_dir = get_cache_dir().await?;

    tokio::task::spawn_blocking(move || -> Result<usize> {
        let mut cleared_count = 0usize;

        for entry in std::fs::read_dir(&cache_dir)
            .with_context(|| format!("Failed to read cache directory: {}", cache_dir.display()))?
        {
            let entry = entry.context("Failed to read cache directory entry")?;
            let path = entry.path();

            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            if !file_name.starts_with(PROVIDER_PLAYLIST_CACHE_PREFIX) {
                continue;
            }

            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove cache file: {}", path.display()))?;
            cleared_count += 1;
        }

        Ok(cleared_count)
    })
    .await
    .map_err(|err| anyhow!("Failed to join provider playlist cache clear task: {err}"))?
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

    #[tokio::test]
    async fn test_write_and_read_cache() {
        let test_file = "test_cache.json";
        let test_data = TestData {
            value: "test".to_string(),
            count: 42,
        };

        // Write cache
        write_cache(test_file, &test_data).await.unwrap();

        // Read cache
        let read_data: Option<TestData> = read_cache(test_file).await.unwrap();
        assert!(read_data.is_some());
        assert_eq!(read_data.unwrap(), test_data);

        // Clean up
        clear_cache(test_file).await.unwrap();
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let test_file = "test_clear.json";
        let test_data = TestData {
            value: "test".to_string(),
            count: 42,
        };

        // Write and then clear
        write_cache(test_file, &test_data).await.unwrap();
        clear_cache(test_file).await.unwrap();

        // Verify it's gone
        let read_data: Option<TestData> = read_cache(test_file).await.unwrap();
        assert!(read_data.is_none());
    }
}
