use keyring::Entry;
/// Configuration management
use rspotify::Token;
use serde::{Deserialize, Serialize};
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

/// Token storage using platform-specific secure storage
///
/// Uses the keyring crate which provides cross-platform secure storage:
/// - **macOS**: Uses the Keychain
/// - **Windows**: Uses the Credential Manager
/// - **Linux**: Uses Secret Service API (e.g., GNOME Keyring, KDE Wallet)
///
/// Tokens are stored securely and encrypted by the operating system.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenStorage {
    /// Spotify token
    pub spotify_token: Option<Token>,
    /// Jellyfin API key (redundant with JellyfinConfig but kept for consistency)
    pub jellyfin_api_key: Option<String>,
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

    /// Load token storage from secure keyring
    pub fn load_tokens() -> Result<TokenStorage, Box<dyn std::error::Error>> {
        tracing::debug!("Loading tokens from keyring");

        match Self::load_tokens_from_keyring() {
            Ok(tokens) => {
                let has_spotify = tokens.spotify_token.is_some();
                let has_jellyfin = tokens.jellyfin_api_key.is_some();
                tracing::debug!(
                    "Loaded from keyring - spotify: {}, jellyfin: {}",
                    has_spotify,
                    has_jellyfin
                );
                Ok(tokens)
            }
            Err(e) => {
                tracing::debug!("No tokens found in keyring: {}", e);
                Ok(TokenStorage::default())
            }
        }
    }

    /// Load tokens directly from keyring
    fn load_tokens_from_keyring() -> Result<TokenStorage, Box<dyn std::error::Error>> {
        let spotify_entry = Entry::new("any-player", "spotify-token")?;
        let jellyfin_entry = Entry::new("any-player", "jellyfin-api-key")?;

        let spotify_token = match spotify_entry.get_password() {
            Ok(json) => {
                tracing::debug!("Found spotify token in keyring, attempting to deserialize");
                match serde_json::from_str(&json) {
                    Ok(token) => {
                        tracing::debug!("Successfully deserialized spotify token");
                        Some(token)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to deserialize spotify token: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                tracing::debug!("No spotify token in keyring: {}", e);
                None
            }
        };

        let jellyfin_api_key = match jellyfin_entry.get_password() {
            Ok(key) => {
                tracing::debug!("Found jellyfin API key in keyring");
                Some(key)
            }
            Err(e) => {
                tracing::debug!("No jellyfin API key in keyring: {}", e);
                None
            }
        };

        // Return tokens (even if both are None)
        Ok(TokenStorage {
            spotify_token,
            jellyfin_api_key,
        })
    }

    /// Save tokens to secure keyring
    ///
    /// # Security Notes
    ///
    /// Tokens are stored using the keyring crate, which provides platform-specific
    /// secure storage:
    /// - **macOS**: Keychain (encrypted by the OS)
    /// - **Windows**: Credential Manager (encrypted by the OS)
    /// - **Linux**: Secret Service API (GNOME Keyring, KDE Wallet, etc.)
    ///
    /// This is significantly more secure than file-based storage as the OS
    /// handles encryption and access control automatically.
    pub fn save_tokens(tokens: &TokenStorage) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(
            "Saving tokens to keyring - spotify: {}, jellyfin: {}",
            tokens.spotify_token.is_some(),
            tokens.jellyfin_api_key.is_some()
        );

        let spotify_entry = Entry::new("any-player", "spotify-token")?;
        let jellyfin_entry = Entry::new("any-player", "jellyfin-api-key")?;

        // Save Spotify token if present
        if let Some(ref token) = tokens.spotify_token {
            let json = serde_json::to_string(token)?;
            spotify_entry.set_password(&json)?;
            tracing::debug!("Successfully saved spotify token to keyring");
        } else {
            // Delete the entry if token is None
            let _ = spotify_entry.delete_credential();
            tracing::debug!("Deleted spotify token from keyring");
        }

        // Save Jellyfin API key if present
        if let Some(ref api_key) = tokens.jellyfin_api_key {
            jellyfin_entry.set_password(api_key)?;
            tracing::debug!("Successfully saved jellyfin API key to keyring");
        } else {
            // Delete the entry if api_key is None
            let _ = jellyfin_entry.delete_credential();
            tracing::debug!("Deleted jellyfin API key from keyring");
        }

        Ok(())
    }

    /// Clear stored tokens from keyring
    pub fn clear_tokens() -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!("Clearing tokens from keyring");

        let spotify_entry = Entry::new("any-player", "spotify-token")?;
        let jellyfin_entry = Entry::new("any-player", "jellyfin-api-key")?;

        // Attempt to delete both entries (ignore errors if they don't exist)
        let _ = spotify_entry.delete_credential();
        let _ = jellyfin_entry.delete_credential();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    #[ignore] // Ignore by default as it requires a functioning keyring service
    fn test_save_and_load_tokens() {
        // Create a test token storage
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("test_api_key".to_string()),
        };

        // Save tokens using keyring
        Config::save_tokens(&tokens).expect("Failed to save tokens");

        // Load tokens using keyring
        let loaded_tokens = Config::load_tokens().expect("Failed to load tokens");

        // Verify
        assert_eq!(
            loaded_tokens.jellyfin_api_key,
            Some("test_api_key".to_string())
        );
        assert!(loaded_tokens.spotify_token.is_none());

        // Cleanup
        let _ = Config::clear_tokens();
    }

    #[test]
    fn test_clear_tokens_nonexistent_file() {
        // This should not panic when tokens don't exist in keyring
        let result = Config::clear_tokens();
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires system keyring service
    fn test_keyring_storage() {
        // Test storing and retrieving from keyring
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("secure_test_key_123".to_string()),
        };

        // Save to keyring
        Config::save_tokens(&tokens).expect("Failed to save to keyring");

        // Load from keyring
        let loaded = Config::load_tokens().expect("Failed to load from keyring");

        assert_eq!(
            loaded.jellyfin_api_key,
            Some("secure_test_key_123".to_string())
        );

        // Cleanup
        Config::clear_tokens().expect("Failed to clear tokens");
    }
}
