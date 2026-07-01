use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::TokuError;

/// Application configuration, persisted as `config.toml` in the data directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TokuConfig {
    /// Default output format: table, json, csv
    pub default_format: String,
    /// Whether to use colors (auto, always, never)
    pub color: String,
    /// Primary metadata source (openlibrary, google)
    pub metadata_source: String,
    /// Sync configuration (optional — absent until `toku sync init`)
    pub sync: Option<SyncConfig>,
    /// Ebook file management configuration (disk organization).
    pub files: FilesConfig,
}

/// File-management configuration stored in `config.toml` under `[files]`.
///
/// Controls how `toku file organize` lays out ebook files on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FilesConfig {
    /// Managed library root. When unset, defaults to `<data_dir>/library`.
    pub library_root: Option<String>,
    /// Path template used to organize files, relative to the library root.
    /// Supports the `{author}`, `{title}`, `{series}`, `{format}`, and `{year}`
    /// tokens, e.g. `{author}/{title}.{format}`.
    pub organize_template: String,
}

/// Default path template: `{author}/{title}.{format}`.
pub const DEFAULT_ORGANIZE_TEMPLATE: &str = "{author}/{title}.{format}";

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            library_root: None,
            organize_template: DEFAULT_ORGANIZE_TEMPLATE.to_string(),
        }
    }
}

/// Sync configuration stored in `config.toml` under `[sync]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncConfig {
    /// Sync server URL
    pub server: String,
    /// Library ID on the server
    pub library_id: String,
    /// This device's ID
    pub device_id: String,
    /// This device's name
    pub device_name: String,
    /// Whether client-side encryption is enabled
    pub encryption: bool,
}

impl Default for TokuConfig {
    fn default() -> Self {
        Self {
            default_format: "table".to_string(),
            color: "auto".to_string(),
            metadata_source: "openlibrary".to_string(),
            sync: None,
            files: FilesConfig::default(),
        }
    }
}

const CONFIG_FILENAME: &str = "config.toml";

impl TokuConfig {
    /// Returns the path to `config.toml` inside the given data directory.
    pub fn config_path(data_dir: &Path) -> PathBuf {
        data_dir.join(CONFIG_FILENAME)
    }

    /// Load configuration from `config.toml` in `data_dir`.
    /// Returns defaults if the file does not exist.
    pub fn load(data_dir: &Path) -> Result<Self, TokuError> {
        let path = Self::config_path(data_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents =
            std::fs::read_to_string(&path).map_err(|e| TokuError::Config(e.to_string()))?;
        let config: TokuConfig =
            toml::from_str(&contents).map_err(|e| TokuError::Config(e.to_string()))?;
        Ok(config)
    }

    /// Save configuration to `config.toml` in `data_dir`.
    /// Creates the data directory if it does not exist.
    pub fn save(&self, data_dir: &Path) -> Result<(), TokuError> {
        std::fs::create_dir_all(data_dir).map_err(|e| TokuError::Config(e.to_string()))?;
        let contents =
            toml::to_string_pretty(self).map_err(|e| TokuError::Config(e.to_string()))?;
        let path = Self::config_path(data_dir);
        std::fs::write(&path, contents).map_err(|e| TokuError::Config(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_sensible_values() {
        let cfg = TokuConfig::default();
        assert_eq!(cfg.default_format, "table");
        assert_eq!(cfg.color, "auto");
        assert_eq!(cfg.metadata_source, "openlibrary");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = std::env::temp_dir().join("toku-test-config-missing");
        // Ensure the directory doesn't have a config file
        let _ = std::fs::remove_file(TokuConfig::config_path(&dir));
        let cfg = TokuConfig::load(&dir).expect("load should succeed");
        assert_eq!(cfg, TokuConfig::default());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("toku-test-config-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        let cfg = TokuConfig {
            default_format: "json".to_string(),
            color: "never".to_string(),
            metadata_source: "google".to_string(),
            sync: None,
            files: FilesConfig::default(),
        };
        cfg.save(&dir).expect("save should succeed");

        let loaded = TokuConfig::load(&dir).expect("load should succeed");
        assert_eq!(loaded, cfg);

        // Clean up
        let _ = std::fs::remove_dir_all(&dir);
    }
}
