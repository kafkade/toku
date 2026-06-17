#[derive(Debug, thiserror::Error)]
pub enum TokuError {
    #[error("invalid book format: {0}")]
    InvalidFormat(String),

    #[error("invalid reading status: {0}")]
    InvalidStatus(String),

    #[error("invalid contributor role: {0}")]
    InvalidRole(String),

    #[error("invalid progress type: {0}")]
    InvalidProgressType(String),

    #[error("invalid duration format: {0}")]
    InvalidDuration(String),

    #[error("invalid tag type: {0}")]
    InvalidTagType(String),

    #[error("invalid pace rating: {0} (expected fast, medium, or slow)")]
    InvalidPaceRating(String),

    #[error("invalid ISBN: {0}")]
    InvalidIsbn(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid transition: cannot move from {from} to {to}")]
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },

    #[error("work not found: {0}")]
    WorkNotFound(String),

    #[error("merge conflict: {0}")]
    MergeConflict(String),

    #[error("invalid filter: {0}")]
    InvalidFilter(String),

    #[error("invalid HLC timestamp: {0}")]
    InvalidHlc(String),

    #[error("invalid entity type: {0}")]
    InvalidEntityType(String),

    #[error("invalid op type: {0}")]
    InvalidOpType(String),

    #[error("crypto error: {0}")]
    Crypto(String),
}
