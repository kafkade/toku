use std::fmt::Write;
use std::path::PathBuf;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

use crate::db::SyncDatabase;
use crate::error::SyncError;

/// Authenticated device identity, injected by the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthDevice {
    pub device_id: String,
    pub library_id: String,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthDevice {
    type Rejection = SyncError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthDevice>()
            .cloned()
            .ok_or(SyncError::Unauthorized)
    }
}

/// SHA-256 hash a string and return the hex-encoded digest.
pub fn sha256_hex(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in hash {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Middleware that validates Bearer tokens and injects `AuthDevice`.
pub async fn require_auth(
    State(db_path): State<PathBuf>,
    mut req: Request,
    next: Next,
) -> Result<Response, SyncError> {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(SyncError::Unauthorized)?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or(SyncError::Unauthorized)?;

    let token_hash = sha256_hex(token);

    let device = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let token_hash = token_hash.clone();
        move || -> Result<AuthDevice, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let mut stmt = db
                .conn
                .prepare("SELECT device_id, library_id FROM devices WHERE auth_token_hash = ?1")?;
            let device = stmt
                .query_row([&token_hash], |row| {
                    Ok(AuthDevice {
                        device_id: row.get(0)?,
                        library_id: row.get(1)?,
                    })
                })
                .map_err(|_| SyncError::Unauthorized)?;

            // Update last_seen
            db.conn.execute(
                "UPDATE devices SET last_seen = datetime('now') WHERE device_id = ?1",
                [&device.device_id],
            )?;

            Ok(device)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    req.extensions_mut().insert(device);
    Ok(next.run(req).await)
}
