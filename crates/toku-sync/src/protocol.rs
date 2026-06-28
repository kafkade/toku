//! Wire-protocol versioning and the version-gate middleware (issue #126).
//!
//! Two protocol generations exist:
//!
//! * **1 — relay**: client-chosen `library_id`, optional single passphrase,
//!   unauthenticated `register`. The original model.
//! * **2 — account**: SRP + Secret Key accounts, the wrapped key hierarchy, and
//!   zero-knowledge ops. The current model.
//!
//! A server advertises [`PROTOCOL_VERSION`] and enforces an instance-configured
//! minimum (`instance_config.min_protocol`). Until an admin migrates the
//! instance the minimum stays at [`PROTOCOL_RELAY`], so old clients keep
//! working. After migration the minimum is bumped to [`PROTOCOL_ACCOUNT`] and
//! pre-account clients are rejected with `426 Upgrade Required`.

use std::path::PathBuf;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::db::SyncDatabase;
use crate::error::SyncError;

/// HTTP header a client sends to declare the protocol version it speaks.
pub const PROTOCOL_HEADER: &str = "x-toku-sync-protocol";

/// Legacy relay protocol: library passphrase, unauthenticated register.
pub const PROTOCOL_RELAY: i64 = 1;
/// Account model: SRP + Secret Key + wrapped key hierarchy.
pub const PROTOCOL_ACCOUNT: i64 = 2;
/// Protocol this server speaks.
pub const PROTOCOL_VERSION: i64 = PROTOCOL_ACCOUNT;

/// Read the configured minimum client protocol, defaulting to [`PROTOCOL_RELAY`]
/// when the column or row is absent (pre-V8 databases).
pub fn min_protocol(db: &SyncDatabase) -> i64 {
    db.conn
        .query_row(
            "SELECT min_protocol FROM instance_config WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(PROTOCOL_RELAY)
}

/// The protocol version the client declared, treating an absent/garbled header
/// as the legacy relay (those clients never sent the header).
fn declared_protocol(req: &Request) -> i64 {
    req.headers()
        .get(PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(PROTOCOL_RELAY)
}

/// Middleware that rejects clients older than the instance's `min_protocol`.
///
/// Applied to data/auth routes; health stays open so clients can probe versions
/// before being turned away.
pub async fn require_protocol(
    State(db_path): State<PathBuf>,
    req: Request,
    next: Next,
) -> Result<Response, SyncError> {
    let declared = declared_protocol(&req);
    let min = tokio::task::spawn_blocking(move || {
        SyncDatabase::open_no_migrate(&db_path)
            .map(|db| min_protocol(&db))
            .unwrap_or(PROTOCOL_RELAY)
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))?;

    if declared < min {
        return Err(SyncError::UpgradeRequired { min });
    }
    Ok(next.run(req).await)
}
