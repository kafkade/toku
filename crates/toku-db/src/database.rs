use std::path::{Path, PathBuf};

use refinery::embed_migrations;
use rusqlite::Connection;

use crate::DbError;

embed_migrations!("migrations");

/// Wraps a SQLite connection with schema migrations and configuration.
pub struct Database {
    pub conn: Connection,
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

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, DbError> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        migrations::runner().run(&mut conn)?;

        Ok(Self { conn })
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
}
