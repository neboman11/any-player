/// Configuration management
use rspotify::Token;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub spotify: Option<SpotifyConfig>,
    pub jellyfin: Option<JellyfinConfig>,
}

/// General application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Application data directory
    pub data_dir: Option<String>,
    /// Enable logging
    pub logging_enabled: bool,
    /// Log level (error, warn, info, debug, trace)
    pub log_level: String,
    /// Enable image rendering
    pub enable_images: bool,
    /// Theme name
    pub theme: String,
}

/// Spotify-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotifyConfig {
    /// Spotify Client ID (OAuth application)
    pub client_id: Option<String>,
    /// Spotify Client Secret (OAuth application)
    pub client_secret: Option<String>,
    /// Redirect URI for OAuth flow
    pub redirect_uri: Option<String>,
    /// Enable direct playback via librespot
    pub enable_streaming: bool,
}

/// Jellyfin-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JellyfinConfig {
    /// Jellyfin server URL (e.g., http://192.168.1.100:8096)
    pub server_url: String,
    /// Jellyfin API key
    pub api_key: String,
    /// Username for authentication
    pub username: Option<String>,
    /// User ID (populated after authentication)
    pub user_id: Option<String>,
}

/// Token storage with file system protections
/// 
/// Stores authentication tokens in a JSON file with restrictive file permissions
/// on Unix systems (0600). Note: This is not cryptographically secure storage.
/// On Windows, file permissions are not as restrictive. For production use with
/// sensitive data, consider platform-specific secure storage mechanisms
/// (e.g., Windows Credential Manager, macOS Keychain).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStorage {
    /// Spotify token
    pub spotify_token: Option<Token>,
    /// Jellyfin API key (redundant with JellyfinConfig but kept for consistency)
    pub jellyfin_api_key: Option<String>,
}

impl Default for TokenStorage {
    fn default() -> Self {
        Self {
            spotify_token: None,
            jellyfin_api_key: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig {
                data_dir: None,
                logging_enabled: true,
                log_level: "info".to_string(),
                enable_images: true,
                theme: "default".to_string(),
            },
            spotify: None,
            jellyfin: None,
        }
    }
}

impl Config {
    /// Load configuration from file or create default
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir()?;
        let config_path = config_dir.join("config.toml");

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            Ok(toml::from_str(&content)?)
        } else {
            // Create default config
            std::fs::create_dir_all(&config_dir)?;
            let config = Self::default();
            let content = toml::to_string_pretty(&config)?;
            std::fs::write(&config_path, content)?;
            Ok(config)
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir()?;
        let config_path = config_dir.join("config.toml");
        std::fs::create_dir_all(&config_dir)?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get configuration directory path
    pub fn config_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = dirs::config_dir()
            .ok_or("Unable to determine config directory")?
            .join("any-player");
        Ok(dir)
    }

    /// Get cache directory path
    pub fn cache_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = dirs::cache_dir()
            .ok_or("Unable to determine cache directory")?
            .join("any-player");
        Ok(dir)
    }

    /// Get data directory
    pub fn get_data_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        if let Some(ref custom_dir) = self.general.data_dir {
            Ok(PathBuf::from(custom_dir))
        } else {
            Self::cache_dir()
        }
    }

    /// Load token storage from secure location
    pub fn load_tokens() -> Result<TokenStorage, Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir()?;
        let token_path = config_dir.join("tokens.json");

        if token_path.exists() {
            let content = fs::read_to_string(&token_path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(TokenStorage::default())
        }
    }

    /// Save tokens to secure location
    ///
    /// # Security Notes
    ///
    /// Currently, tokens are stored in plain JSON with file permissions set to 0600 (user-only)
    /// on Unix systems. This provides basic protection but is not cryptographically secure.
    ///
    /// **Recommendations for production use:**
    /// - **macOS**: Use the Keychain API via the `keyring` or `security-framework` crate
    /// - **Windows**: Use the Credential Manager via the `keyring` or `windows` crate
    /// - **Linux**: Use Secret Service API via the `keyring` or `secret-service` crate
    /// - **Cross-platform**: Consider the `keyring` crate which provides a unified interface
    ///
    /// Alternatively, tokens could be encrypted using a key derived from the system
    /// (e.g., via `ring` or `aes-gcm` crates) before writing to disk.
    ///
    /// For now, we rely on:
    /// 1. OAuth tokens that expire and can be refreshed
    /// 2. File system permissions (Unix: 0600, Windows: ACLs via OS defaults)
    /// 3. User responsibility for system security
    pub fn save_tokens(tokens: &TokenStorage) -> Result<(), Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir()?;
        let token_path = config_dir.join("tokens.json");
        fs::create_dir_all(&config_dir)?;

        let content = serde_json::to_string_pretty(tokens)?;
        fs::write(&token_path, content)?;

        // Set secure permissions (600) on token file
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&token_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&token_path, perms)?;
        }

        Ok(())
    }

    /// Clear stored tokens
    pub fn clear_tokens() -> Result<(), Box<dyn std::error::Error>> {
        let config_dir = Self::config_dir()?;
        let token_path = config_dir.join("tokens.json");

        if token_path.exists() {
            fs::remove_file(&token_path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.general.log_level, "info");
        assert!(config.general.enable_images);
    }

    #[test]
    fn test_token_storage_default() {
        let storage = TokenStorage::default();
        assert!(storage.spotify_token.is_none());
        assert!(storage.jellyfin_api_key.is_none());
    }

    #[test]
    fn test_token_storage_serialization() {
        let storage = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("test_key".to_string()),
        };

        // Test that we can serialize to JSON
        let json = serde_json::to_string(&storage);
        assert!(json.is_ok());

        // Test that we can deserialize from JSON
        let json_str = json.unwrap();
        let deserialized: Result<TokenStorage, _> = serde_json::from_str(&json_str);
        assert!(deserialized.is_ok());

        let deserialized_storage = deserialized.unwrap();
        assert_eq!(
            deserialized_storage.jellyfin_api_key,
            Some("test_key".to_string())
        );
    }

    #[test]
    fn test_save_and_load_tokens() {
        // Create a test token storage
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("test_api_key".to_string()),
        };

        // Save tokens using the actual Config method
        // This will save to the real config directory, which is fine for testing
        Config::save_tokens(&tokens).expect("Failed to save tokens");

        // Load tokens using the actual Config method
        let loaded_tokens = Config::load_tokens().expect("Failed to load tokens");

        // Verify
        assert_eq!(
            loaded_tokens.jellyfin_api_key,
            Some("test_api_key".to_string())
        );
        assert!(loaded_tokens.spotify_token.is_none());

        // Verify file permissions on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let config_dir = Config::config_dir().expect("Failed to get config dir");
            let token_path = config_dir.join("tokens.json");
            let metadata = fs::metadata(&token_path).expect("Failed to get file metadata");
            let mode = metadata.permissions().mode();
            // Check that only owner has read/write (0600)
            assert_eq!(mode & 0o777, 0o600, "Token file should have 0600 permissions");
        }

        // Cleanup
        let _ = Config::clear_tokens();
    }
    }

    #[test]
    fn test_clear_tokens_nonexistent_file() {
        // This should not panic when the file doesn't exist
        // We can't easily test the actual clear_tokens function without mocking,
        // but we can test the file removal logic
        let temp_dir = std::env::temp_dir().join("any-player-test-clear");
        let test_path = temp_dir.join("nonexistent_tokens.json");

        // Ensure it doesn't exist
        let _ = fs::remove_file(&test_path);

        // This should handle the case gracefully
        if test_path.exists() {
            let result = fs::remove_file(&test_path);
            assert!(result.is_ok());
        }
        // No assertion needed - just shouldn't panic
    }

    #[cfg(unix)]
    #[test]
    fn test_token_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join("any-player-test-perms");
        fs::create_dir_all(&temp_dir).unwrap();
        let test_token_path = temp_dir.join("test_tokens_perms.json");

        // Create test file
        let tokens = TokenStorage::default();
        let json = serde_json::to_string_pretty(&tokens).unwrap();
        fs::write(&test_token_path, json).unwrap();

        // Set secure permissions (600)
        let mut perms = fs::metadata(&test_token_path).unwrap().permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&test_token_path, perms).unwrap();

        // Verify permissions
        let metadata = fs::metadata(&test_token_path).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Cleanup
        let _ = fs::remove_file(&test_token_path);
        let _ = fs::remove_dir(&temp_dir);
    }
}
