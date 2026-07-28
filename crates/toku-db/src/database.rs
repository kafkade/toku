use std::path::{Path, PathBuf};

use refinery::embed_migrations;
use rusqlite::Connection;

use crate::DbError;

#[cfg(feature = "sqlcipher")]
use toku_core::SyncKey;

embed_migrations!("migrations");

/// Wraps a SQLite connection with schema migrations and configuration.
pub struct Database {
    pub conn: Connection,
}

/// Process-global at-rest DB unlock key (ADR-016 D4/D5).
///
/// When the `sqlcipher` feature is on and the local database is encrypted, the
/// entry point (CLI / web server) resolves the key **once** and installs it here
/// for the process lifetime. The `*_default` constructors then transparently
/// open the database keyed, so the many request-time open sites need no
/// key-threading. The base `open`/`open_no_migrate`/`open_encrypted`
/// constructors never consult this global — migration and tests depend on their
/// exact, unambiguous behavior.
#[cfg(feature = "sqlcipher")]
mod key_state {
    use std::sync::OnceLock;

    use toku_core::SyncKey;

    static DB_KEY: OnceLock<SyncKey> = OnceLock::new();

    /// Install the process unlock key. Returns `Err` if one is already set.
    pub fn set_process_db_key(key: SyncKey) -> Result<(), &'static str> {
        DB_KEY.set(key).map_err(|_| "process DB key already set")
    }

    /// The installed process unlock key, if any.
    pub fn process_db_key() -> Option<&'static SyncKey> {
        DB_KEY.get()
    }
}

#[cfg(feature = "sqlcipher")]
pub use key_state::{process_db_key, set_process_db_key};

/// Apply a raw 256-bit key to a freshly opened SQLCipher connection and confirm
/// it decrypts the header.
///
/// The key is passed as a raw hex blob (`PRAGMA key = "x'<64hex>'"`) so
/// SQLCipher uses it directly and does **not** run its own weaker PBKDF2 — Toku
/// derives the key with Argon2id (`SyncKey::derive`) itself. This must be the
/// **first** statement after `Connection::open`, before WAL / foreign_keys /
/// migrations, because SQLCipher needs the key before the header is read.
///
/// A wrong key surfaces as a clean [`DbError::Encryption`] on the probe query
/// (SQLCipher returns `SQLITE_NOTADB`) — never a panic, never a partial open.
#[cfg(feature = "sqlcipher")]
fn apply_key(conn: &Connection, key: &SyncKey) -> Result<(), DbError> {
    use std::fmt::Write as _;

    let mut hex = String::with_capacity(64);
    for byte in key.as_exported_bytes() {
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    // `PRAGMA key` does not accept bound parameters; the value is our own hex of
    // a fixed-length key, so there is no untrusted interpolation here.
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .map_err(|e| DbError::Encryption(format!("failed to apply key: {e}")))?;

    // Force the header to be read/decrypted. A wrong key fails here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|_| {
            DbError::Encryption(
                "incorrect passphrase or not an encrypted Toku database".to_string(),
            )
        })?;
    Ok(())
}

impl Database {
    /// Open (or create) the database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DbError::Io(e.to_string()))?;
        }

        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::runner().run(&mut conn)?;

        Ok(Self { conn })
    }

    /// Open the database without running migrations.
    ///
    /// Use this for request-time reads after migrations have already run
    /// at startup via [`Database::open`].
    pub fn open_no_migrate(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, DbError> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::runner().run(&mut conn)?;

        Ok(Self { conn })
    }

    /// Open (or create) an **encrypted** database and run migrations.
    ///
    /// Mirrors [`Database::open`] but applies `key` via SQLCipher's `PRAGMA key`
    /// as the first statement, before WAL / foreign_keys / migrations. A wrong
    /// key yields [`DbError::Encryption`] rather than a panic or partial open.
    ///
    /// Only available when the `sqlcipher` feature is enabled.
    #[cfg(feature = "sqlcipher")]
    pub fn open_encrypted(path: &Path, key: &SyncKey) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DbError::Io(e.to_string()))?;
        }

        let mut conn = Connection::open(path)?;
        apply_key(&conn, key)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::runner().run(&mut conn)?;

        Ok(Self { conn })
    }

    /// Open an **encrypted** database without running migrations.
    ///
    /// Mirrors [`Database::open_no_migrate`] but applies `key` first. Use for
    /// request-time reads after migrations have already run at startup via
    /// [`Database::open_encrypted`].
    ///
    /// Only available when the `sqlcipher` feature is enabled.
    #[cfg(feature = "sqlcipher")]
    pub fn open_no_migrate_encrypted(path: &Path, key: &SyncKey) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        apply_key(&conn, key)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    /// Open (or create) the database and run migrations, transparently keyed
    /// when a process unlock key is installed (see [`set_process_db_key`]).
    ///
    /// This is the constructor application entry points should use. With the
    /// `sqlcipher` feature **off**, it is identical to [`Database::open`].
    pub fn open_default(path: &Path) -> Result<Self, DbError> {
        #[cfg(feature = "sqlcipher")]
        if let Some(key) = key_state::process_db_key() {
            return Self::open_encrypted(path, key);
        }
        Self::open(path)
    }

    /// Open the database without migrations, transparently keyed when a process
    /// unlock key is installed. With `sqlcipher` off, identical to
    /// [`Database::open_no_migrate`].
    pub fn open_no_migrate_default(path: &Path) -> Result<Self, DbError> {
        #[cfg(feature = "sqlcipher")]
        if let Some(key) = key_state::process_db_key() {
            return Self::open_no_migrate_encrypted(path, key);
        }
        Self::open_no_migrate(path)
    }

    /// Returns the default data directory for the current platform.
    pub fn default_data_dir() -> Result<PathBuf, DbError> {
        if let Ok(dir) = std::env::var("TOKU_DATA_DIR") {
            return Ok(PathBuf::from(dir));
        }

        let proj_dirs = directories::ProjectDirs::from("com", "kafkade", "toku")
            .ok_or_else(|| DbError::Io("could not determine data directory".to_string()))?;

        Ok(proj_dirs.data_dir().to_path_buf())
    }

    /// Returns the default database file path.
    pub fn default_db_path() -> Result<PathBuf, DbError> {
        Ok(Self::default_data_dir()?.join("toku.db"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_runs_migrations() {
        let db = Database::open_in_memory().unwrap();
        // Verify the books table exists
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM books", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn open_file_creates_and_migrates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM books", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(feature = "sqlcipher")]
    mod encrypted {
        use super::*;
        use toku_core::SyncKey;

        fn test_key(pass: &str) -> SyncKey {
            let salt: [u8; 16] = [9; 16];
            SyncKey::derive(pass, &salt).unwrap()
        }

        #[test]
        fn open_encrypted_roundtrip() {
            let tmp = tempfile::TempDir::new().unwrap();
            let db_path = tmp.path().join("enc.db");
            let key = test_key("correct horse");

            {
                let db = Database::open_encrypted(&db_path, &key).unwrap();
                db.conn
                    .execute(
                        "INSERT INTO books (id, title, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?3)",
                        (
                            "01972123-abcd-7000-8000-000000000001",
                            "Dune",
                            "2026-07-27T00:00:00Z",
                        ),
                    )
                    .unwrap();
            }

            // Reopen without migrating, same key, data survives.
            let db = Database::open_no_migrate_encrypted(&db_path, &key).unwrap();
            let title: String = db
                .conn
                .query_row("SELECT title FROM books LIMIT 1", [], |r| r.get(0))
                .unwrap();
            assert_eq!(title, "Dune");
        }

        #[test]
        fn wrong_key_fails_cleanly() {
            let tmp = tempfile::TempDir::new().unwrap();
            let db_path = tmp.path().join("enc.db");
            let key = test_key("correct horse");
            let wrong = test_key("battery staple");

            {
                let _db = Database::open_encrypted(&db_path, &key).unwrap();
            }

            let err = Database::open_no_migrate_encrypted(&db_path, &wrong);
            match err {
                Err(DbError::Encryption(msg)) => {
                    assert!(msg.contains("incorrect passphrase"), "got: {msg}");
                }
                Err(other) => panic!("expected Encryption error, got {other:?}"),
                Ok(_) => panic!("expected wrong key to fail"),
            }
        }

        #[test]
        fn plaintext_db_opened_with_key_fails_cleanly() {
            let tmp = tempfile::TempDir::new().unwrap();
            let db_path = tmp.path().join("plain.db");
            let key = test_key("correct horse");

            // Create a plaintext DB.
            {
                let _db = Database::open(&db_path).unwrap();
            }

            let err = Database::open_no_migrate_encrypted(&db_path, &key);
            match err {
                Err(DbError::Encryption(_)) => {}
                Err(other) => panic!("expected Encryption error, got {other:?}"),
                Ok(_) => panic!("expected plaintext-with-key to fail"),
            }
        }
    }
}
