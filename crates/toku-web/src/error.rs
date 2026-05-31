use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("database error: {0}")]
    Database(#[from] toku_db::DbError),

    #[error("import error: {0}")]
    Import(#[from] toku_import::ImportError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let status = match &self {
            WebError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebError::Import(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebError::NotFound(_) => StatusCode::NOT_FOUND,
            WebError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
