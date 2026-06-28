use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::models::ErrorBody;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("plaintext payload rejected: {0}")]
    PlaintextRejected(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("too many failed authentication attempts; retry after {retry_after}")]
    RateLimited { retry_after: String },

    #[error("account locked until {until}")]
    AccountLocked { until: String },

    #[error("client too old: minimum sync protocol is {min}; upgrade Toku to continue")]
    UpgradeRequired { min: i64 },

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for SyncError {
    fn into_response(self) -> Response {
        let status = match &self {
            SyncError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SyncError::Migration(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SyncError::NotFound(_) => StatusCode::NOT_FOUND,
            SyncError::Unauthorized => StatusCode::UNAUTHORIZED,
            SyncError::Forbidden(_) => StatusCode::FORBIDDEN,
            SyncError::BadRequest(_) => StatusCode::BAD_REQUEST,
            SyncError::PlaintextRejected(_) => StatusCode::UNPROCESSABLE_ENTITY,
            SyncError::Conflict(_) => StatusCode::CONFLICT,
            SyncError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            SyncError::AccountLocked { .. } => StatusCode::from_u16(423).unwrap(),
            SyncError::UpgradeRequired { .. } => StatusCode::UPGRADE_REQUIRED,
            SyncError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            error: self.to_string(),
        };
        (status, axum::Json(body)).into_response()
    }
}
