use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;

use base64::Engine as _;

static KEYRING_INIT: Once = Once::new();

/// Whether the OS keychain should be bypassed in favour of the encrypted-at-rest
/// file fallback. Controlled by the environment so headless servers and CI (which
/// have no usable keychain, or must not pollute the developer's keychain) can opt
/// out: set `TOKU_TOKEN_STORE=file` or `TOKU_DISABLE_KEYCHAIN=1`.
fn keychain_disabled() -> bool {
    matches!(
        std::env::var("TOKU_TOKEN_STORE").as_deref(),
        Ok("file") | Ok("FILE")
    ) || matches!(
        std::env::var("TOKU_DISABLE_KEYCHAIN").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// Register the OS-native credential store as the keyring default.
///
/// keyring 4.x decouples the API (`keyring-core`) from the platform stores, so
/// a default store must be registered before any [`keyring_core::Entry`] is
/// created. This runs once; if no OS keychain is available the entry calls
/// below fail gracefully and callers fall back to file storage.
fn ensure_keyring_store() {
    KEYRING_INIT.call_once(|| {
        if let Some(store) = native_store() {
            keyring_core::set_default_store(store);
        }
    });
}

/// Build the platform-native credential store, if one is available.
#[cfg(target_os = "linux")]
fn native_store() -> Option<std::sync::Arc<keyring_core::CredentialStore>> {
    let store: std::sync::Arc<keyring_core::CredentialStore> =
        zbus_secret_service_keyring_store::Store::new().ok()?;
    Some(store)
}

#[cfg(target_os = "macos")]
fn native_store() -> Option<std::sync::Arc<keyring_core::CredentialStore>> {
    let store: std::sync::Arc<keyring_core::CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().ok()?;
    Some(store)
}

#[cfg(target_os = "windows")]
fn native_store() -> Option<std::sync::Arc<keyring_core::CredentialStore>> {
    let store: std::sync::Arc<keyring_core::CredentialStore> =
        windows_native_keyring_store::Store::new().ok()?;
    Some(store)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_store() -> Option<std::sync::Arc<keyring_core::CredentialStore>> {
    None
}

/// Stores sync auth tokens using the OS keychain or a local file fallback.
///
/// Tokens are keyed by the normalized server URL.
pub struct TokenStore {
    data_dir: PathBuf,
}

const KEYRING_SERVICE: &str = "toku-sync";
const KEYRING_SERVICE_SYNCKEY: &str = "toku-sync-key";

impl TokenStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Store a token for the given server URL.
    pub fn store(&self, server_url: &str, token: &str) -> anyhow::Result<()> {
        let key = normalize_url(server_url);

        // Try OS keychain first (unless disabled by env)
        if !keychain_disabled() {
            ensure_keyring_store();
            match keyring_core::Entry::new(KEYRING_SERVICE, &key) {
                Ok(entry) => match entry.set_password(token) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        eprintln!(
                            "warning: could not store token in OS keychain ({e}), using file fallback"
                        );
                    }
                },
                Err(e) => {
                    eprintln!("warning: keychain unavailable ({e}), using file fallback");
                }
            }
        }

        self.store_file(&key, token)
    }

    /// Store an SRP session token along with its expiry timestamp.
    /// Uses the same storage path as `store` so `load` returns the session token.
    pub fn store_session(
        &self,
        server_url: &str,
        session_token: &str,
        expires_at: &str,
    ) -> anyhow::Result<()> {
        // Store the token itself using the standard path (OS keychain / file).
        self.store(server_url, session_token)?;
        // Store the expiry separately in the file fallback (informational only).
        let key = normalize_url(server_url);
        self.store_file(&format!("{key}:session_expires"), expires_at)
    }

    /// Load the stored session token expiry, if present.
    #[allow(dead_code)]
    pub fn load_session_expiry(&self, server_url: &str) -> anyhow::Result<Option<String>> {
        let key = normalize_url(server_url);
        self.load_file(&format!("{key}:session_expires"))
    }

    /// Load a token for the given server URL.
    pub fn load(&self, server_url: &str) -> anyhow::Result<Option<String>> {
        let key = normalize_url(server_url);

        // Try OS keychain first (unless disabled by env)
        if !keychain_disabled() {
            ensure_keyring_store();
            if let Ok(entry) = keyring_core::Entry::new(KEYRING_SERVICE, &key) {
                match entry.get_password() {
                    Ok(token) => return Ok(Some(token)),
                    Err(keyring_core::Error::NoEntry) => {}
                    Err(_) => {}
                }
            }
        }

        // Fall back to file
        self.load_file(&key)
    }

    /// Delete a token for the given server URL.
    pub fn delete(&self, server_url: &str) -> anyhow::Result<()> {
        let key = normalize_url(server_url);

        // Try OS keychain (unless disabled by env)
        if !keychain_disabled() {
            ensure_keyring_store();
            if let Ok(entry) = keyring_core::Entry::new(KEYRING_SERVICE, &key) {
                let _ = entry.delete_credential();
            }
        }

        // Also remove from file fallback
        self.delete_file(&key)
    }

    /// Store the derived sync encryption key in the OS keychain.
    pub fn store_sync_key(&self, server_url: &str, key_bytes: &[u8]) -> anyhow::Result<()> {
        let key = normalize_url(server_url);
        let encoded = base64::engine::general_purpose::STANDARD.encode(key_bytes);

        if !keychain_disabled() {
            ensure_keyring_store();
            if let Ok(entry) = keyring_core::Entry::new(KEYRING_SERVICE_SYNCKEY, &key) {
                match entry.set_password(&encoded) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        eprintln!(
                            "warning: could not store sync key in OS keychain ({e}), using file fallback"
                        );
                    }
                }
            }
        }

        self.store_file(&format!("{key}:synckey"), &encoded)
    }

    /// Load the derived sync encryption key from the OS keychain.
    pub fn load_sync_key(&self, server_url: &str) -> anyhow::Result<Option<Vec<u8>>> {
        use base64::Engine;
        let key = normalize_url(server_url);

        if !keychain_disabled() {
            ensure_keyring_store();
            if let Ok(entry) = keyring_core::Entry::new(KEYRING_SERVICE_SYNCKEY, &key) {
                match entry.get_password() {
                    Ok(encoded) => {
                        let bytes = base64::engine::general_purpose::STANDARD.decode(&encoded)?;
                        return Ok(Some(bytes));
                    }
                    Err(keyring_core::Error::NoEntry) => {}
                    Err(_) => {}
                }
            }
        }

        // File fallback
        match self.load_file(&format!("{key}:synckey"))? {
            Some(encoded) => {
                let bytes = base64::engine::general_purpose::STANDARD.decode(&encoded)?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    /// Delete the stored sync encryption key.
    #[allow(dead_code)]
    pub fn delete_sync_key(&self, server_url: &str) -> anyhow::Result<()> {
        let key = normalize_url(server_url);

        if !keychain_disabled() {
            ensure_keyring_store();
            if let Ok(entry) = keyring_core::Entry::new(KEYRING_SERVICE_SYNCKEY, &key) {
                let _ = entry.delete_credential();
            }
        }

        self.delete_file(&format!("{key}:synckey"))
    }

    // ── File fallback ───────────────────────────────────────────────────

    fn tokens_path(&self) -> PathBuf {
        self.data_dir.join("sync").join("tokens.json")
    }

    fn read_tokens_file(&self) -> HashMap<String, String> {
        let path = self.tokens_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    fn write_tokens_file(&self, tokens: &HashMap<String, String>) -> anyhow::Result<()> {
        let path = self.tokens_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(tokens)?;
        std::fs::write(&path, json)?;

        // On Unix, restrict file permissions to owner-only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    fn store_file(&self, key: &str, token: &str) -> anyhow::Result<()> {
        let mut tokens = self.read_tokens_file();
        tokens.insert(key.to_string(), token.to_string());
        self.write_tokens_file(&tokens)
    }

    fn load_file(&self, key: &str) -> anyhow::Result<Option<String>> {
        let tokens = self.read_tokens_file();
        Ok(tokens.get(key).cloned())
    }

    fn delete_file(&self, key: &str) -> anyhow::Result<()> {
        let mut tokens = self.read_tokens_file();
        if tokens.remove(key).is_some() {
            self.write_tokens_file(&tokens)?;
        }
        Ok(())
    }
}

/// Normalize a server URL to a consistent key for token storage.
fn normalize_url(url: &str) -> String {
    let mut s = url.to_lowercase();
    // Strip trailing slashes
    while s.ends_with('/') {
        s.pop();
    }
    // Strip trailing /api or /api/v1 paths
    for suffix in &["/api/v1", "/api"] {
        if s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_strips_trailing_slashes() {
        assert_eq!(
            normalize_url("https://sync.example.com/"),
            "https://sync.example.com"
        );
        assert_eq!(
            normalize_url("https://sync.example.com///"),
            "https://sync.example.com"
        );
    }

    #[test]
    fn normalize_url_strips_api_paths() {
        assert_eq!(
            normalize_url("https://sync.example.com/api/v1"),
            "https://sync.example.com"
        );
        assert_eq!(
            normalize_url("https://sync.example.com/api"),
            "https://sync.example.com"
        );
    }

    #[test]
    fn normalize_url_lowercases() {
        assert_eq!(
            normalize_url("HTTPS://Sync.Example.COM"),
            "https://sync.example.com"
        );
    }

    #[test]
    fn file_fallback_roundtrip() {
        let dir = std::env::temp_dir().join(format!("toku-token-test-{}", uuid::Uuid::now_v7()));
        let store = TokenStore::new(&dir);

        store.store_file("https://a.com", "token-a").unwrap();
        store.store_file("https://b.com", "token-b").unwrap();

        assert_eq!(
            store.load_file("https://a.com").unwrap(),
            Some("token-a".into())
        );
        assert_eq!(
            store.load_file("https://b.com").unwrap(),
            Some("token-b".into())
        );

        store.delete_file("https://a.com").unwrap();
        assert_eq!(store.load_file("https://a.com").unwrap(), None);
        assert_eq!(
            store.load_file("https://b.com").unwrap(),
            Some("token-b".into())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
