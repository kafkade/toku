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
    /// OPDS server configuration (optional HTTP Basic auth).
    pub opds: OpdsConfig,
}

/// OPDS server configuration stored in `config.toml` under `[opds]`.
///
/// Controls the optional HTTP Basic authentication that guards the OPDS
/// catalog served by `toku opds`. Authentication is enabled only when *both*
/// `username` and `password_hash` are set — otherwise the catalog is served
/// without a login prompt (the local-network default).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct OpdsConfig {
    /// HTTP Basic auth username. `None` disables authentication.
    pub username: Option<String>,
    /// Salted password hash, formatted `sha256$<salt_hex>$<hash_hex>`.
    /// `None` disables authentication.
    pub password_hash: Option<String>,
}

impl OpdsConfig {
    /// True when both a username and password hash are configured, meaning
    /// the OPDS server should require HTTP Basic authentication.
    pub fn auth_enabled(&self) -> bool {
        self.username.as_deref().is_some_and(|u| !u.is_empty())
            && self.password_hash.as_deref().is_some_and(|h| !h.is_empty())
    }

    /// Hash a plaintext password with a fresh random 16-byte salt, producing a
    /// `sha256$<salt_hex>$<hash_hex>` string suitable for `password_hash`.
    pub fn hash_password(password: &str) -> String {
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).expect("system RNG must be available");
        format!(
            "sha256${}${}",
            hex_encode(&salt),
            sha256_salted(&salt, password)
        )
    }

    /// Constant-time verification of a plaintext password against the stored
    /// `sha256$<salt_hex>$<hash_hex>` hash. Returns false on any malformed hash.
    pub fn verify_password(&self, password: &str) -> bool {
        let Some(hash) = self.password_hash.as_deref() else {
            return false;
        };
        verify_password_hash(hash, password)
    }
}

/// Verify a plaintext password against a `sha256$<salt_hex>$<hash_hex>` string,
/// in constant time. Returns false for any malformed or non-matching input.
pub fn verify_password_hash(stored: &str, password: &str) -> bool {
    let mut parts = stored.splitn(3, '$');
    let (Some(scheme), Some(salt_hex), Some(hash_hex)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if scheme != "sha256" {
        return false;
    }
    let Some(salt) = hex_decode(salt_hex) else {
        return false;
    };
    let expected = sha256_salted(&salt, password);
    constant_time_eq(expected.as_bytes(), hash_hex.as_bytes())
}

/// SHA-256 of `salt || password`, hex-encoded.
fn sha256_salted(salt: &[u8], password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Length-independent constant-time byte comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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
            opds: OpdsConfig::default(),
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
            opds: OpdsConfig::default(),
        };
        cfg.save(&dir).expect("save should succeed");

        let loaded = TokuConfig::load(&dir).expect("load should succeed");
        assert_eq!(loaded, cfg);

        // Clean up
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opds_auth_disabled_by_default() {
        let cfg = TokuConfig::default();
        assert!(!cfg.opds.auth_enabled());
    }

    #[test]
    fn opds_auth_enabled_only_with_both_fields() {
        let mut opds = OpdsConfig {
            username: Some("reader".to_string()),
            password_hash: None,
        };
        assert!(!opds.auth_enabled());
        opds.password_hash = Some(OpdsConfig::hash_password("s3cret"));
        assert!(opds.auth_enabled());
    }

    #[test]
    fn opds_password_hash_roundtrips() {
        let opds = OpdsConfig {
            username: Some("reader".to_string()),
            password_hash: Some(OpdsConfig::hash_password("correct horse battery staple")),
        };
        assert!(opds.verify_password("correct horse battery staple"));
        assert!(!opds.verify_password("wrong password"));
    }

    #[test]
    fn opds_password_hash_uses_random_salt() {
        let a = OpdsConfig::hash_password("same");
        let b = OpdsConfig::hash_password("same");
        // Different salts ⇒ different stored hashes, but both verify.
        assert_ne!(a, b);
        assert!(verify_password_hash(&a, "same"));
        assert!(verify_password_hash(&b, "same"));
    }

    #[test]
    fn opds_verify_rejects_malformed_hash() {
        assert!(!verify_password_hash("", "x"));
        assert!(!verify_password_hash("notahash", "x"));
        assert!(!verify_password_hash("md5$aa$bb", "x"));
        assert!(!verify_password_hash("sha256$zz$bb", "x"));
    }

    #[test]
    fn opds_config_roundtrips_through_toml() {
        let dir = std::env::temp_dir().join("toku-test-config-opds");
        let _ = std::fs::remove_dir_all(&dir);

        let cfg = TokuConfig {
            opds: OpdsConfig {
                username: Some("reader".to_string()),
                password_hash: Some(OpdsConfig::hash_password("hunter2")),
            },
            ..TokuConfig::default()
        };
        cfg.save(&dir).expect("save should succeed");

        let loaded = TokuConfig::load(&dir).expect("load should succeed");
        assert_eq!(loaded, cfg);
        assert!(loaded.opds.verify_password("hunter2"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
