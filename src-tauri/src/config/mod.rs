/// Configuration management
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
}
