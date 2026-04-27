#[derive(Debug, thiserror::Error)]
pub enum TokuError {
    #[error("invalid book format: {0}")]
    InvalidFormat(String),

    #[error("invalid reading status: {0}")]
    InvalidStatus(String),

    #[error("invalid contributor role: {0}")]
    InvalidRole(String),

    #[error("invalid ISBN: {0}")]
    InvalidIsbn(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid transition: cannot move from {from} to {to}")]
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },
}
