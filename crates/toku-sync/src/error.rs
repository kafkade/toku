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
            SyncError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            error: self.to_string(),
        };
        (status, axum::Json(body)).into_response()
    }
}
