#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("book not found for ISBN: {0}")]
    NotFound(String),

    #[error("failed to parse API response: {0}")]
    Parse(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
