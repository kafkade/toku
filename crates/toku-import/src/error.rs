#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("database error: {0}")]
    Db(#[from] toku_db::DbError),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("import error: {0}")]
    Other(String),
}
