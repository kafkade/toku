use std::path::Path;

use refinery::embed_migrations;
use rusqlite::Connection;

use crate::error::SyncError;

embed_migrations!("migrations");

/// Server-side SQLite database for sync op storage.
pub struct SyncDatabase {
    pub conn: Connection,
}

impl SyncDatabase {
    /// Open (or create) the database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self, SyncError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SyncError::Internal(format!("failed to create data dir: {e}")))?;
        }

        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;

        migrations::runner()
            .run(&mut conn)
            .map_err(|e| SyncError::Migration(e.to_string()))?;

        Ok(Self { conn })
    }

    /// Open without running migrations (for per-request connections).
    pub fn open_no_migrate(path: &Path) -> Result<Self, SyncError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        Ok(Self { conn })
    }
}
