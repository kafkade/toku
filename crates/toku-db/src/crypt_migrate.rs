//! Plaintext ↔ SQLCipher database migration via `sqlcipher_export()`.
//!
//! Only compiled when the `sqlcipher` feature is enabled (ADR-016 D6). These
//! functions copy a whole database into a new file with a different key state,
//! leaving the source untouched so the caller can verify the destination before
//! swapping files.

use std::path::Path;

use rusqlite::Connection;

use crate::DbError;
use toku_core::SyncKey;

/// Render a 256-bit key as SQLCipher's raw-key hex literal (`x'<64hex>'`).
fn key_hex_literal(key: &SyncKey) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in key.as_exported_bytes() {
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("x'{hex}'")
}

/// Copy a **plaintext** database at `src` into a new **encrypted** database at
/// `dst`, keyed with `key`. `dst` must not already exist. `src` is left
/// unchanged.
pub fn encrypt_to(src: &Path, dst: &Path, key: &SyncKey) -> Result<(), DbError> {
    if dst.exists() {
        return Err(DbError::Encryption(format!(
            "destination already exists: {}",
            dst.display()
        )));
    }
    let conn = Connection::open(src)?;
    let dst_str = dst
        .to_str()
        .ok_or_else(|| DbError::Encryption("destination path is not valid UTF-8".to_string()))?;
    let key_lit = key_hex_literal(key);

    conn.execute_batch(&format!(
        "ATTACH DATABASE '{}' AS toku_target KEY \"{}\";\n\
         SELECT sqlcipher_export('toku_target');\n\
         DETACH DATABASE toku_target;",
        sql_escape_single_quotes(dst_str),
        key_lit,
    ))
    .map_err(|e| DbError::Encryption(format!("encrypt export failed: {e}")))?;
    Ok(())
}

/// Copy an **encrypted** database at `src` (opened with `key`) into a new
/// **plaintext** database at `dst`. `dst` must not already exist. `src` is left
/// unchanged.
pub fn decrypt_to(src: &Path, dst: &Path, key: &SyncKey) -> Result<(), DbError> {
    if dst.exists() {
        return Err(DbError::Encryption(format!(
            "destination already exists: {}",
            dst.display()
        )));
    }
    let conn = Connection::open(src)?;
    // Key the source first (validates the passphrase before exporting).
    conn.execute_batch(&format!("PRAGMA key = \"{}\";", key_hex_literal(key)))
        .map_err(|e| DbError::Encryption(format!("failed to apply key: {e}")))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|_| {
            DbError::Encryption(
                "incorrect passphrase or not an encrypted Toku database".to_string(),
            )
        })?;

    let dst_str = dst
        .to_str()
        .ok_or_else(|| DbError::Encryption("destination path is not valid UTF-8".to_string()))?;
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{}' AS toku_target KEY '';\n\
         SELECT sqlcipher_export('toku_target');\n\
         DETACH DATABASE toku_target;",
        sql_escape_single_quotes(dst_str),
    ))
    .map_err(|e| DbError::Encryption(format!("decrypt export failed: {e}")))?;
    Ok(())
}

/// Escape single quotes in a SQLite string literal (path interpolated into
/// `ATTACH DATABASE '...'`, which does not accept bound parameters).
fn sql_escape_single_quotes(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn test_key(pass: &str) -> SyncKey {
        let salt: [u8; 16] = [3; 16];
        SyncKey::derive(pass, &salt).unwrap()
    }

    fn seed(path: &Path) {
        let db = Database::open(path).unwrap();
        db.conn
            .execute(
                "INSERT INTO books (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                (
                    "01972123-abcd-7000-8000-000000000009",
                    "Neuromancer",
                    "2026-07-27T00:00:00Z",
                ),
            )
            .unwrap();
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plain = tmp.path().join("plain.db");
        let enc = tmp.path().join("enc.db");
        let back = tmp.path().join("back.db");
        let key = test_key("roundtrip-pass");

        seed(&plain);
        encrypt_to(&plain, &enc, &key).unwrap();

        // Encrypted file opens with the key and holds the data.
        let db = Database::open_no_migrate_encrypted(&enc, &key).unwrap();
        let title: String = db
            .conn
            .query_row("SELECT title FROM books LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "Neuromancer");
        drop(db);

        // Decrypt back to plaintext.
        decrypt_to(&enc, &back, &key).unwrap();
        let db = Database::open_no_migrate(&back).unwrap();
        let title: String = db
            .conn
            .query_row("SELECT title FROM books LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "Neuromancer");
    }

    #[test]
    fn decrypt_with_wrong_key_fails_cleanly() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plain = tmp.path().join("plain.db");
        let enc = tmp.path().join("enc.db");
        let back = tmp.path().join("back.db");

        seed(&plain);
        encrypt_to(&plain, &enc, &test_key("right")).unwrap();

        let err = decrypt_to(&enc, &back, &test_key("wrong")).unwrap_err();
        assert!(matches!(err, DbError::Encryption(_)));
        assert!(!back.exists() || std::fs::metadata(&back).unwrap().len() == 0);
    }

    #[test]
    fn refuses_existing_destination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plain = tmp.path().join("plain.db");
        let enc = tmp.path().join("enc.db");
        seed(&plain);
        std::fs::write(&enc, b"existing").unwrap();
        let err = encrypt_to(&plain, &enc, &test_key("x")).unwrap_err();
        assert!(matches!(err, DbError::Encryption(_)));
    }
}
