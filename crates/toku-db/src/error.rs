#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(#[from] refinery::Error),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),
}
