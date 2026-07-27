#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("database error: {0}")]
    Database(#[from] toku_db::DbError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error(
        "unsupported backup format version {0}. The flat v1 backup was export-only and \
         is superseded by ADR-012; re-run `toku export backup` from your live library to \
         produce a restorable v{1} archive."
    )]
    UnsupportedVersion(u32, u32),

    #[error(
        "backup format version {0} is newer than this build supports (max v{1}); \
         upgrade Toku to restore it"
    )]
    FutureVersion(u32, u32),

    #[error("encryption error: {0}")]
    Crypto(String),

    #[error("malformed backup: {0}")]
    Malformed(String),

    #[error("export error: {0}")]
    Other(String),
}
