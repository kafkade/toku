use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::auth::{AuthDevice, AuthUser, generate_token, sha256_hex};
use crate::db::SyncDatabase;
use crate::error::SyncError;
use crate::models::{
    AccountDeviceListResponse, AccountDeviceSummary, AccountKeysResponse, DeviceApprovalRequest,
    DeviceApprovalsConfigResponse, DeviceResponse, DeviceSessionResponse, DownloadSnapshotResponse,
    EnrollDeviceRequest, EnrollDeviceResponse, HealthResponse, OpPayload, PullQuery, PullResponse,
    PushRequest, PushResponse, RegisterRequest, RegisterResponse, RegistrationConfigResponse,
    RekeyRequest, RekeyResponse, SetDeviceApprovalsRequest, SetRegistrationRequest,
    SetUserStatusRequest, UploadSnapshotRequest, UploadSnapshotResponse, UserListResponse,
    UserSummary,
};

const MAX_BATCH_SIZE: usize = 1000;
const MAX_BATCH_SIZE_SQL: i64 = MAX_BATCH_SIZE as i64;

// ── Zero-knowledge payload enforcement (issue #121) ───────────────────────────

/// Returns `true` if `payload` is a well-formed encrypted envelope.
///
/// A valid envelope is a JSON object containing **exactly** the envelope keys
/// (`ev`, `alg`, `nonce`, `ciphertext`, `aad`) with the expected primitive
/// types. Requiring an exact key set prevents a client from smuggling
/// plaintext fields alongside the ciphertext.
fn is_encrypted_envelope(payload: &serde_json::Value) -> bool {
    let Some(obj) = payload.as_object() else {
        return false;
    };
    const KEYS: [&str; 5] = ["ev", "alg", "nonce", "ciphertext", "aad"];
    if obj.len() != KEYS.len() || !KEYS.iter().all(|k| obj.contains_key(*k)) {
        return false;
    }
    obj.get("ev").is_some_and(serde_json::Value::is_u64)
        && obj.get("alg").is_some_and(serde_json::Value::is_string)
        && obj.get("nonce").is_some_and(serde_json::Value::is_string)
        && obj
            .get("ciphertext")
            .is_some_and(serde_json::Value::is_string)
        && obj.get("aad").is_some_and(serde_json::Value::is_string)
}

/// Enforce zero-knowledge for a synced op payload.
///
/// Accepts either an encrypted envelope object or JSON `null` (content-free
/// ops such as deletes). Any other shape — notably a plaintext `fields`
/// object — is rejected so the server can never read user content. The op
/// metadata (ids, hlc, entity/op type) intentionally stays cleartext; the
/// `payload` must always be ciphertext.
fn require_ciphertext_payload(payload: &serde_json::Value) -> Result<(), SyncError> {
    if payload.is_null() || is_encrypted_envelope(payload) {
        Ok(())
    } else {
        Err(SyncError::PlaintextRejected(
            "op payload must be an encrypted envelope; plaintext is not allowed in hosted mode"
                .into(),
        ))
    }
}

/// Reject a batch outright if any op carries a plaintext payload.
fn require_ciphertext_batch(ops: &[OpPayload]) -> Result<(), SyncError> {
    for op in ops {
        require_ciphertext_payload(&op.payload).map_err(|_| {
            SyncError::PlaintextRejected(format!(
                "op {} carries a plaintext payload; hosted mode requires client-side encryption",
                op.op_id
            ))
        })?;
    }
    Ok(())
}

/// Enforce zero-knowledge for an uploaded snapshot blob.
///
/// The snapshot must be a serialized encrypted envelope (ciphertext), never a
/// plaintext `LibrarySnapshot`.
fn require_ciphertext_snapshot(snapshot_json: &str) -> Result<(), SyncError> {
    let value: serde_json::Value = serde_json::from_str(snapshot_json).map_err(|_| {
        SyncError::PlaintextRejected(
            "snapshot must be an encrypted envelope; plaintext is not allowed in hosted mode"
                .into(),
        )
    })?;
    if is_encrypted_envelope(&value) {
        Ok(())
    } else {
        Err(SyncError::PlaintextRejected(
            "snapshot must be an encrypted envelope; plaintext is not allowed in hosted mode"
                .into(),
        ))
    }
}

// ── Health ──────────────────────────────────────────────────────────────────

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ── Register ────────────────────────────────────────────────────────────────

pub async fn register(
    State(db_path): State<PathBuf>,
    headers: axum::http::HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, SyncError> {
    if req.device_name.is_empty() {
        return Err(SyncError::BadRequest("device_name is required".into()));
    }
    if req.library_id.is_empty() {
        return Err(SyncError::BadRequest("library_id is required".into()));
    }

    // Extract optional Bearer token (present when adding a device to an SRP library).
    let bearer_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string());

    // Only generate a static token for passwordless libraries; SRP libraries use sessions.
    let static_token = generate_token();
    let static_token_hash = sha256_hex(&static_token);
    let device_id = uuid::Uuid::now_v7().to_string();

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = req.library_id.clone();
        let device_name = req.device_name.clone();
        let device_id = device_id.clone();
        let static_token_hash = static_token_hash.clone();
        let salt = req.salt.clone();
        let bearer_token = bearer_token.clone();
        move || -> Result<RegisterResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // If this library has SRP credentials, only accept authenticated device additions.
            let is_srp_library: bool = db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM accounts WHERE library_id = ?1",
                    [&library_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            // For SRP libraries: validate the session token (must match the library).
            // Keep the session hash so we can rebind it after the device row is created.
            let srp_session_hash: Option<String> = if is_srp_library {
                let session_token = bearer_token.clone().ok_or(SyncError::Unauthorized)?;
                let session_hash = sha256_hex(&session_token);
                let auth_library_id: String = db
                    .conn
                    .query_row(
                        "SELECT library_id FROM sessions
                         WHERE session_token_hash = ?1 AND expires_at > datetime('now')",
                        [&session_hash],
                        |row| row.get(0),
                    )
                    .map_err(|_| SyncError::Unauthorized)?;
                if auth_library_id != library_id {
                    return Err(SyncError::Forbidden(
                        "session token belongs to a different library".into(),
                    ));
                }
                Some(session_hash)
            } else {
                None
            };

            // Hard-gate open registration on account-managed instances (issue #120).
            //
            // Once any account exists the instance is "managed": the legacy
            // unauthenticated relay path (non-SRP library, no session) is closed
            // so that guessing a `library_id` no longer grants access. Callers
            // must enroll through POST /api/v1/devices/enroll instead. SRP-session
            // adds (validated above) and user-session-authenticated adds still
            // work; zero-account (bootstrap / offline-relay) instances are
            // unaffected, preserving the legacy passwordless flow.
            if !is_srp_library {
                let account_managed: bool = db
                    .conn
                    .query_row("SELECT COUNT(*) FROM users", [], |row| row.get::<_, i64>(0))
                    .unwrap_or(0)
                    > 0;
                if account_managed
                    && crate::auth::resolve_user_owner(&db, bearer_token.as_deref()).is_none()
                {
                    return Err(SyncError::Forbidden(
                        "open device registration is disabled on this account-managed instance; \
                         authenticate and enroll via POST /api/v1/devices/enroll"
                            .into(),
                    ));
                }
            }

            // Auto-create library if it doesn't exist.
            db.conn.execute(
                "INSERT OR IGNORE INTO libraries (id, created_at) VALUES (?1, datetime('now'))",
                [&library_id],
            )?;

            // Establish the library salt on first encrypted registration.
            if let Some(ref salt) = salt {
                db.conn.execute(
                    "UPDATE libraries SET salt = ?1 WHERE id = ?2 AND salt IS NULL",
                    rusqlite::params![salt, library_id],
                )?;
            }

            // SRP libraries don't use static auth tokens; store empty string.
            let token_hash_to_store = if is_srp_library {
                String::new()
            } else {
                static_token_hash
            };

            db.conn.execute(
                "INSERT INTO devices (device_id, library_id, device_name, auth_token_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![device_id, library_id, device_name, token_hash_to_store],
            )?;

            // Ownership stamping (issue #119): if a valid user-session bearer is
            // present, mark the library + device as owned by that user. Legacy
            // unauthenticated registrations leave user_id NULL (unowned).
            if let Some(owner_id) = crate::auth::resolve_user_owner(&db, bearer_token.as_deref()) {
                db.conn.execute(
                    "UPDATE libraries SET user_id = ?1 WHERE id = ?2 AND user_id IS NULL",
                    rusqlite::params![owner_id, library_id],
                )?;
                db.conn.execute(
                    "UPDATE devices SET user_id = ?1 WHERE device_id = ?2",
                    rusqlite::params![owner_id, device_id],
                )?;
            }

            // Rebind the session to the newly-created device. The FK constraint on
            // sessions.device_id requires the device row to exist first.
            if let Some(ref session_hash) = srp_session_hash {
                let _ = db.conn.execute(
                    "UPDATE sessions SET device_id = ?1 WHERE session_token_hash = ?2",
                    rusqlite::params![device_id, session_hash],
                );
            }

            // Return the appropriate token: static for passwordless, empty for SRP.
            Ok(RegisterResponse {
                device_id,
                library_id,
                auth_token: String::new(), // filled in below for passwordless
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    // For passwordless libraries, return the static bearer token.
    // For SRP libraries, return empty string (session token was issued by /auth/verify).
    let auth_token = if bearer_token.is_none() {
        static_token
    } else {
        String::new()
    };

    Ok(Json(RegisterResponse { auth_token, ..resp }))
}

// ── Devices ─────────────────────────────────────────────────────────────────

pub async fn list_devices(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
) -> Result<Json<Vec<DeviceResponse>>, SyncError> {
    let devices = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        move || -> Result<Vec<DeviceResponse>, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let mut stmt = db.conn.prepare(
                "SELECT device_id, device_name, last_seen, created_at
                 FROM devices WHERE library_id = ?1 ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map([&library_id], |row| {
                    Ok(DeviceResponse {
                        device_id: row.get(0)?,
                        device_name: row.get(1)?,
                        last_seen: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(devices))
}

pub async fn delete_device(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
    Path(target_id): Path<String>,
) -> Result<Json<serde_json::Value>, SyncError> {
    if target_id == device.device_id {
        return Err(SyncError::Forbidden("cannot delete your own device".into()));
    }

    tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        move || -> Result<(), SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Only allow deleting devices in the same library
            let affected = db.conn.execute(
                "DELETE FROM devices WHERE device_id = ?1 AND library_id = ?2",
                rusqlite::params![target_id, library_id],
            )?;
            if affected == 0 {
                return Err(SyncError::NotFound(format!("device {target_id} not found")));
            }

            // Clean up cursors for the deleted device
            db.conn
                .execute("DELETE FROM cursors WHERE device_id = ?1", [&target_id])?;

            Ok(())
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ── Authenticated device enrollment (issue #120) ─────────────────────────────

/// Device session token TTL (hours). Mirrors the SRP session TTL.
const DEVICE_SESSION_TTL_HOURS: i64 = 24;

/// Insert a fresh device session token row and return `(token, expires_at)`.
/// Runs inside a blocking task with an open connection.
fn issue_device_session(
    db: &SyncDatabase,
    device_id: &str,
    library_id: &str,
) -> Result<(String, String), SyncError> {
    let token = generate_token();
    let token_hash = sha256_hex(&token);
    let expires_at: String = db
        .conn
        .query_row(
            "SELECT datetime('now', ?1)",
            [format!("+{DEVICE_SESSION_TTL_HOURS} hours")],
            |row| row.get(0),
        )
        .map_err(|e| SyncError::Internal(format!("datetime query failed: {e}")))?;
    db.conn.execute(
        "INSERT INTO sessions
         (session_token_hash, device_id, library_id, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
        rusqlite::params![token_hash, device_id, library_id, expires_at],
    )?;
    Ok((token, expires_at))
}

/// `POST /api/v1/devices/enroll`
///
/// Enroll a device under the authenticated account. Requires a user-session
/// bearer (issued by the account SRP flow, which already proves possession of
/// the password + Secret Key). The device is bound to a library the user owns —
/// no library is auto-created for an unauthenticated caller.
///
/// When the instance requires device approvals and the target library already
/// has an active device, the new device is recorded as `pending` (no token) and
/// must be approved by an existing trusted device before it can sync.
pub async fn enroll_device(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Json(req): Json<EnrollDeviceRequest>,
) -> Result<Json<EnrollDeviceResponse>, SyncError> {
    if req.device_name.is_empty() {
        return Err(SyncError::BadRequest("device_name is required".into()));
    }

    let device_id = uuid::Uuid::now_v7().to_string();

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        let device_id = device_id.clone();
        let requested_library = req.library_id.clone();
        let device_name = req.device_name.clone();
        let encryption_salt = req.encryption_salt.clone();
        let device_public_key = req.device_public_key.clone();
        move || -> Result<EnrollDeviceResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let tx = db.conn.unchecked_transaction()?;

            // Resolve the target library, enforcing ownership.
            let library_id = match requested_library {
                Some(lib) if !lib.is_empty() => {
                    let owner: Option<Option<String>> = tx
                        .query_row(
                            "SELECT user_id FROM libraries WHERE id = ?1",
                            [&lib],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .ok();
                    match owner {
                        // Existing library: must be owned by this user.
                        Some(Some(owner_id)) if owner_id == user_id => lib,
                        Some(_) => {
                            return Err(SyncError::Forbidden(
                                "you are not authorized to enroll into this library".into(),
                            ));
                        }
                        // Library does not exist yet: create it owned by the user.
                        None => {
                            tx.execute(
                                "INSERT INTO libraries (id, created_at, user_id)
                                 VALUES (?1, datetime('now'), ?2)",
                                rusqlite::params![lib, user_id],
                            )?;
                            lib
                        }
                    }
                }
                // No library specified: mint a fresh one owned by the user.
                _ => {
                    let lib = uuid::Uuid::now_v7().to_string();
                    tx.execute(
                        "INSERT INTO libraries (id, created_at, user_id)
                         VALUES (?1, datetime('now'), ?2)",
                        rusqlite::params![lib, user_id],
                    )?;
                    lib
                }
            };

            // Establish the library salt on first encrypted enrollment.
            if let Some(ref salt) = encryption_salt {
                tx.execute(
                    "UPDATE libraries SET salt = ?1 WHERE id = ?2 AND salt IS NULL",
                    rusqlite::params![salt, library_id],
                )?;
            }

            // Decide whether this device needs approval. Approval only applies
            // when the toggle is on AND the library already has an active device
            // (the first device cannot be approved by anyone).
            let approvals_required: bool = tx
                .query_row(
                    "SELECT device_approvals_required FROM instance_config WHERE id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let active_devices: i64 = tx.query_row(
                "SELECT COUNT(*) FROM devices WHERE library_id = ?1 AND status = 'active'",
                [&library_id],
                |row| row.get(0),
            )?;
            let status = if approvals_required && active_devices > 0 {
                "pending"
            } else {
                "active"
            };

            // SRP/account libraries don't use static auth tokens.
            tx.execute(
                "INSERT INTO devices
                 (device_id, library_id, device_name, auth_token_hash, user_id,
                  status, device_public_key, created_at)
                 VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, datetime('now'))",
                rusqlite::params![
                    device_id,
                    library_id,
                    device_name,
                    user_id,
                    status,
                    device_public_key,
                ],
            )?;

            let (session_token, expires_at) = if status == "active" {
                let (token, exp) = issue_device_session(&db, &device_id, &library_id)?;
                (Some(token), Some(exp))
            } else {
                (None, None)
            };

            tx.commit()?;

            Ok(EnrollDeviceResponse {
                device_id,
                library_id,
                status: status.to_string(),
                session_token,
                expires_at,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// `POST /api/v1/devices/{id}/approval`
///
/// Approve or reject a `pending` device. Scoped strictly to the authenticated
/// user: only devices the user owns can be acted on. Approving flips the device
/// to `active` (it can then mint a session token); rejecting marks it `rejected`
/// and revokes any sessions.
pub async fn approve_device(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Path(target_id): Path<String>,
    Json(req): Json<DeviceApprovalRequest>,
) -> Result<Json<AccountDeviceSummary>, SyncError> {
    let approve = match req.decision.as_str() {
        "approve" => true,
        "reject" => false,
        _ => {
            return Err(SyncError::BadRequest(
                "decision must be 'approve' or 'reject'".into(),
            ));
        }
    };

    let summary = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        let target_id = target_id.clone();
        move || -> Result<AccountDeviceSummary, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Load the device, scoped to the authenticated owner.
            let (library_id, device_name, status, last_seen, created_at): (
                String,
                String,
                String,
                Option<String>,
                String,
            ) = db
                .conn
                .query_row(
                    "SELECT library_id, device_name, status, last_seen, created_at
                     FROM devices WHERE device_id = ?1 AND user_id = ?2",
                    rusqlite::params![target_id, user_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .map_err(|_| SyncError::NotFound(format!("device {target_id} not found")))?;

            if status != "pending" {
                return Err(SyncError::Conflict(format!(
                    "device is '{status}', not pending approval"
                )));
            }

            let new_status = if approve { "active" } else { "rejected" };
            db.conn.execute(
                "UPDATE devices SET status = ?1 WHERE device_id = ?2",
                rusqlite::params![new_status, target_id],
            )?;
            if !approve {
                // Defensive: drop any sessions for a rejected device.
                db.conn
                    .execute("DELETE FROM sessions WHERE device_id = ?1", [&target_id])?;
            }

            Ok(AccountDeviceSummary {
                device_id: target_id,
                library_id,
                device_name,
                status: new_status.to_string(),
                last_seen,
                created_at,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(summary))
}

/// `POST /api/v1/devices/{id}/session`
///
/// Mint a device session token for an `active` device owned by the authenticated
/// user. Used by a previously `pending` device to obtain its token after
/// approval, and as a refresh path for an existing device.
pub async fn create_device_session(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Path(target_id): Path<String>,
) -> Result<Json<DeviceSessionResponse>, SyncError> {
    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        let target_id = target_id.clone();
        move || -> Result<DeviceSessionResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            let (library_id, status): (String, String) = db
                .conn
                .query_row(
                    "SELECT library_id, status FROM devices
                     WHERE device_id = ?1 AND user_id = ?2",
                    rusqlite::params![target_id, user_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| SyncError::NotFound(format!("device {target_id} not found")))?;

            match status.as_str() {
                "active" => {}
                "pending" => {
                    return Err(SyncError::Forbidden(
                        "device is pending approval by an existing trusted device".into(),
                    ));
                }
                _ => {
                    return Err(SyncError::Forbidden(
                        "device enrollment was rejected".into(),
                    ));
                }
            }

            let (session_token, expires_at) = issue_device_session(&db, &target_id, &library_id)?;

            Ok(DeviceSessionResponse {
                device_id: target_id,
                library_id,
                session_token,
                expires_at,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// `GET /api/v1/account/devices`
///
/// List the devices owned by the authenticated user, across all of their
/// libraries. Strictly user-scoped — never exposes another account's devices.
pub async fn list_account_devices(
    State(db_path): State<PathBuf>,
    user: AuthUser,
) -> Result<Json<AccountDeviceListResponse>, SyncError> {
    let devices = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        move || -> Result<Vec<AccountDeviceSummary>, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let mut stmt = db.conn.prepare(
                "SELECT device_id, library_id, device_name, status, last_seen, created_at
                 FROM devices WHERE user_id = ?1 ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map([&user_id], |row| {
                    Ok(AccountDeviceSummary {
                        device_id: row.get(0)?,
                        library_id: row.get(1)?,
                        device_name: row.get(2)?,
                        status: row.get(3)?,
                        last_seen: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(AccountDeviceListResponse { devices }))
}

/// `GET /api/v1/account/keys`
///
/// Return the authenticated account's key bundle so a new device can recover the
/// shared library data key the zero-knowledge way (issue #143):
/// SRP login → `GET /account/keys` → `AccountKeys::unlock_data_key(password, secret_key)`.
///
/// The four fields are opaque ciphertext / public-key material persisted verbatim
/// at signup — the server never sees the password, Secret Key, or plaintext data
/// key. An account missing any field (e.g. a row created before #143, or a
/// partial signup) is reported as `409 Conflict` rather than emitting JSON nulls,
/// keeping the response contract a set of required non-null strings.
pub async fn account_keys(
    State(db_path): State<PathBuf>,
    user: AuthUser,
) -> Result<Json<AccountKeysResponse>, SyncError> {
    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        move || -> Result<AccountKeysResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let (kdf_params, account_public_key, wrapped_private_key, wrapped_data_key): (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = db
                .conn
                .query_row(
                    "SELECT kdf_params, account_public_key, wrapped_private_key, wrapped_data_key
                     FROM users WHERE id = ?1",
                    [&user_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| SyncError::Unauthorized)?;

            match (
                kdf_params,
                account_public_key,
                wrapped_private_key,
                wrapped_data_key,
            ) {
                (
                    Some(kdf_params),
                    Some(account_public_key),
                    Some(wrapped_private_key),
                    Some(wrapped_data_key),
                ) => Ok(AccountKeysResponse {
                    kdf_params,
                    account_public_key,
                    wrapped_private_key,
                    wrapped_data_key,
                }),
                _ => Err(SyncError::Conflict(
                    "account key bundle not provisioned".into(),
                )),
            }
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// `DELETE /api/v1/account/devices/{id}`
///
/// Deregister a device owned by the authenticated user. Cleans up the device's
/// sessions and cursors. Strictly user-scoped.
pub async fn delete_account_device(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Path(target_id): Path<String>,
) -> Result<Json<serde_json::Value>, SyncError> {
    tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let user_id = user.user_id.clone();
        let target_id = target_id.clone();
        move || -> Result<(), SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Confirm ownership first (404 if absent or owned by someone else),
            // then remove child rows before the device to satisfy FK constraints.
            let owned: i64 = db.conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE device_id = ?1 AND user_id = ?2",
                rusqlite::params![target_id, user_id],
                |row| row.get(0),
            )?;
            if owned == 0 {
                return Err(SyncError::NotFound(format!("device {target_id} not found")));
            }

            db.conn
                .execute("DELETE FROM sessions WHERE device_id = ?1", [&target_id])?;
            db.conn
                .execute("DELETE FROM cursors WHERE device_id = ?1", [&target_id])?;
            db.conn.execute(
                "DELETE FROM devices WHERE device_id = ?1 AND user_id = ?2",
                rusqlite::params![target_id, user_id],
            )?;
            Ok(())
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ── Push ────────────────────────────────────────────────────────────────────

pub async fn push_ops(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, SyncError> {
    if req.ops.is_empty() {
        return Err(SyncError::BadRequest("ops array is empty".into()));
    }
    if req.ops.len() > MAX_BATCH_SIZE {
        return Err(SyncError::BadRequest(format!(
            "batch too large: {} ops (max {MAX_BATCH_SIZE})",
            req.ops.len()
        )));
    }

    // Zero-knowledge: reject the batch if any op carries a plaintext payload.
    require_ciphertext_batch(&req.ops)?;

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        let device_id = device.device_id.clone();
        let ops = req.ops;
        move || -> Result<PushResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Reject pushes while a rekey is in progress
            let locked: bool = db
                .conn
                .query_row(
                    "SELECT rekey_in_progress FROM libraries WHERE id = ?1",
                    [&library_id],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if locked {
                return Err(SyncError::BadRequest(
                    "library is being re-keyed; try again shortly".into(),
                ));
            }

            let tx = db.conn.unchecked_transaction()?;

            let mut accepted = 0usize;
            let mut duplicates = 0usize;

            for op in &ops {
                let payload_str = serde_json::to_string(&op.payload)
                    .map_err(|e| SyncError::BadRequest(format!("invalid payload: {e}")))?;

                let result = tx.execute(
                    "INSERT OR IGNORE INTO ops (op_id, library_id, device_id, hlc, entity_type, entity_id, op_type, payload, received_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                    rusqlite::params![
                        op.op_id,
                        library_id,
                        op.device_id,
                        op.hlc,
                        op.entity_type,
                        op.entity_id,
                        op.op_type,
                        payload_str,
                    ],
                )?;

                if result > 0 {
                    accepted += 1;
                } else {
                    duplicates += 1;
                }
            }

            // Update push cursor to the last accepted op
            let new_cursor = ops.last().map(|op| op.op_id.clone());
            if let Some(ref cursor_op_id) = new_cursor {
                tx.execute(
                    "INSERT INTO cursors (device_id, cursor_type, op_id, updated_at)
                     VALUES (?1, 'push', ?2, datetime('now'))
                     ON CONFLICT (device_id, cursor_type) DO UPDATE SET
                       op_id = excluded.op_id,
                       updated_at = excluded.updated_at",
                    rusqlite::params![device_id, cursor_op_id],
                )?;
            }

            tx.commit()?;

            Ok(PushResponse {
                accepted,
                duplicates,
                new_cursor,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Pull ────────────────────────────────────────────────────────────────────

pub async fn pull_ops(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
    Query(query): Query<PullQuery>,
) -> Result<Json<PullResponse>, SyncError> {
    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        let device_id = device.device_id.clone();
        let since = query.since;
        move || -> Result<PullResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Fetch one extra to determine has_more
            let fetch_limit = MAX_BATCH_SIZE_SQL + 1;

            let all_ops: Vec<OpPayload> = if let Some(ref since_op_id) = since {
                // Get the HLC of the cursor op to filter from
                let cursor_hlc: Option<String> = db
                    .conn
                    .query_row(
                        "SELECT hlc FROM ops WHERE op_id = ?1 AND library_id = ?2",
                        rusqlite::params![since_op_id, library_id],
                        |row| row.get(0),
                    )
                    .ok();

                match cursor_hlc {
                    Some(hlc) => {
                        let mut stmt = db.conn.prepare(
                            "SELECT op_id, device_id, hlc, entity_type, entity_id, op_type, payload
                             FROM ops
                             WHERE library_id = ?1
                               AND device_id != ?2
                               AND (hlc > ?3 OR (hlc = ?3 AND op_id > ?4))
                             ORDER BY hlc, op_id
                             LIMIT ?5",
                        )?;
                        stmt.query_map(
                            rusqlite::params![library_id, device_id, hlc, since_op_id, fetch_limit],
                            row_to_op,
                        )?
                        .collect::<Result<Vec<_>, _>>()?
                    }
                    None => {
                        return Err(SyncError::BadRequest(format!(
                            "cursor op_id not found: {since_op_id}"
                        )));
                    }
                }
            } else {
                // No cursor — return all ops from the beginning (excluding own)
                let mut stmt = db.conn.prepare(
                    "SELECT op_id, device_id, hlc, entity_type, entity_id, op_type, payload
                     FROM ops
                     WHERE library_id = ?1 AND device_id != ?2
                     ORDER BY hlc, op_id
                     LIMIT ?3",
                )?;
                stmt.query_map(
                    rusqlite::params![library_id, device_id, fetch_limit],
                    row_to_op,
                )?
                .collect::<Result<Vec<_>, _>>()?
            };

            let has_more = all_ops.len() > MAX_BATCH_SIZE;
            let ops: Vec<OpPayload> = all_ops.into_iter().take(MAX_BATCH_SIZE).collect();

            let cursor = ops.last().map(|op| op.op_id.clone());

            // Update pull cursor
            if let Some(ref cursor_op_id) = cursor {
                db.conn.execute(
                    "INSERT INTO cursors (device_id, cursor_type, op_id, updated_at)
                     VALUES (?1, 'pull', ?2, datetime('now'))
                     ON CONFLICT (device_id, cursor_type) DO UPDATE SET
                       op_id = excluded.op_id,
                       updated_at = excluded.updated_at",
                    rusqlite::params![device_id, cursor_op_id],
                )?;
            }

            Ok(PullResponse {
                ops,
                cursor,
                has_more,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Snapshot / Salt ─────────────────────────────────────────────────────────

// ── Snapshot ────────────────────────────────────────────────────────────────

/// Download the latest snapshot for this library.
pub async fn download_snapshot(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
) -> Result<Json<DownloadSnapshotResponse>, SyncError> {
    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        move || -> Result<DownloadSnapshotResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let row: Option<(String, String, String, String)> = db
                .conn
                .query_row(
                    "SELECT snapshot_json, hlc_at_snapshot, created_at, created_by_device
                     FROM snapshots
                     WHERE library_id = ?1
                     ORDER BY created_at DESC
                     LIMIT 1",
                    [&library_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .ok();

            match row {
                Some((snapshot_json, hlc, created_at, device_id)) => Ok(DownloadSnapshotResponse {
                    snapshot_json,
                    hlc_at_snapshot: hlc,
                    created_at,
                    created_by_device: device_id,
                }),
                None => Err(SyncError::NotFound("no snapshot available".into())),
            }
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// Upload a snapshot and prune ops older than the snapshot HLC.
pub async fn upload_snapshot(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
    Json(req): Json<UploadSnapshotRequest>,
) -> Result<Json<UploadSnapshotResponse>, SyncError> {
    if req.snapshot_json.is_empty() {
        return Err(SyncError::BadRequest("snapshot_json is required".into()));
    }
    if req.hlc_at_snapshot.is_empty() {
        return Err(SyncError::BadRequest("hlc_at_snapshot is required".into()));
    }

    // Zero-knowledge: the snapshot blob must be an encrypted envelope.
    require_ciphertext_snapshot(&req.snapshot_json)?;

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        let device_id = device.device_id.clone();
        let snapshot_json = req.snapshot_json;
        let hlc = req.hlc_at_snapshot;
        move || -> Result<UploadSnapshotResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let tx = db.conn.unchecked_transaction()?;

            // Store the snapshot
            tx.execute(
                "INSERT INTO snapshots (library_id, snapshot_json, hlc_at_snapshot, created_by_device, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![library_id, snapshot_json, hlc, device_id],
            )?;

            // Prune ops older than the snapshot HLC
            let pruned = tx.execute(
                "DELETE FROM ops WHERE library_id = ?1 AND hlc <= ?2",
                rusqlite::params![library_id, hlc],
            )?;

            // Keep only the latest snapshot per library
            tx.execute(
                "DELETE FROM snapshots
                 WHERE library_id = ?1
                   AND id != (SELECT id FROM snapshots WHERE library_id = ?1 ORDER BY created_at DESC LIMIT 1)",
                [&library_id],
            )?;

            tx.commit()?;

            Ok(UploadSnapshotResponse {
                ops_pruned: pruned,
                hlc_at_snapshot: hlc,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// Get the library salt (needed for key derivation on new devices).
pub async fn get_salt(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
) -> Result<Json<serde_json::Value>, SyncError> {
    let salt = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        move || -> Result<Option<String>, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let salt: Option<String> = db
                .conn
                .query_row(
                    "SELECT salt FROM libraries WHERE id = ?1",
                    [&library_id],
                    |row| row.get(0),
                )
                .map_err(|_| SyncError::NotFound("library not found".into()))?;
            Ok(salt)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(serde_json::json!({ "salt": salt })))
}

/// Pull ALL ops for this library (including own device's ops).
/// Used during re-keying when the client needs to decrypt and re-encrypt everything.
pub async fn pull_all_ops(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
) -> Result<Json<PullResponse>, SyncError> {
    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        move || -> Result<PullResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let mut stmt = db.conn.prepare(
                "SELECT op_id, device_id, hlc, entity_type, entity_id, op_type, payload
                 FROM ops
                 WHERE library_id = ?1
                 ORDER BY hlc, op_id",
            )?;
            let ops: Vec<OpPayload> = stmt
                .query_map([&library_id], row_to_op)?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(PullResponse {
                cursor: ops.last().map(|op| op.op_id.clone()),
                has_more: false,
                ops,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Rekey ───────────────────────────────────────────────────────────────────

pub async fn rekey(
    State(db_path): State<PathBuf>,
    device: AuthDevice,
    Json(req): Json<RekeyRequest>,
) -> Result<Json<RekeyResponse>, SyncError> {
    if req.new_salt.is_empty() {
        return Err(SyncError::BadRequest("new_salt is required".into()));
    }
    if req.ops.is_empty() {
        return Err(SyncError::BadRequest("ops array is empty".into()));
    }

    // Zero-knowledge: re-keyed ops must remain ciphertext.
    require_ciphertext_batch(&req.ops)?;

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = device.library_id.clone();
        let new_salt = req.new_salt;
        let ops = req.ops;
        move || -> Result<RekeyResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Set rekey lock
            db.conn.execute(
                "UPDATE libraries SET rekey_in_progress = 1 WHERE id = ?1",
                [&library_id],
            )?;

            // Run the replacement in a transaction so failure rolls back
            let result = (|| -> Result<RekeyResponse, SyncError> {
                let tx = db.conn.unchecked_transaction()?;

                // Delete all existing ops for this library
                let deleted: usize = tx.execute(
                    "DELETE FROM ops WHERE library_id = ?1",
                    [&library_id],
                )?;
                let _ = deleted;

                // Insert the re-encrypted ops
                let mut inserted = 0usize;
                for op in &ops {
                    let payload_str = serde_json::to_string(&op.payload)
                        .map_err(|e| SyncError::BadRequest(format!("invalid payload: {e}")))?;

                    tx.execute(
                        "INSERT INTO ops (op_id, library_id, device_id, hlc, entity_type, entity_id, op_type, payload, received_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                        rusqlite::params![
                            op.op_id,
                            library_id,
                            op.device_id,
                            op.hlc,
                            op.entity_type,
                            op.entity_id,
                            op.op_type,
                            payload_str,
                        ],
                    )?;
                    inserted += 1;
                }

                // Update salt
                tx.execute(
                    "UPDATE libraries SET salt = ?1 WHERE id = ?2",
                    rusqlite::params![new_salt, library_id],
                )?;

                // Invalidate all cursors for this library's devices
                tx.execute(
                    "DELETE FROM cursors WHERE device_id IN (
                         SELECT device_id FROM devices WHERE library_id = ?1
                     )",
                    [&library_id],
                )?;

                tx.commit()?;

                Ok(RekeyResponse {
                    ops_replaced: inserted,
                    new_salt: new_salt.clone(),
                })
            })();

            // Always release the rekey lock, even on failure
            let _ = db.conn.execute(
                "UPDATE libraries SET rekey_in_progress = 0 WHERE id = ?1",
                [&library_id],
            );

            result
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn row_to_op(row: &rusqlite::Row) -> rusqlite::Result<OpPayload> {
    let payload_str: String = row.get(6)?;
    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::String(payload_str));

    Ok(OpPayload {
        op_id: row.get(0)?,
        device_id: row.get(1)?,
        hlc: row.get(2)?,
        entity_type: row.get(3)?,
        entity_id: row.get(4)?,
        op_type: row.get(5)?,
        payload,
    })
}

// ── Admin (issue #119) ───────────────────────────────────────────────────────

/// Reject non-admin callers. Admin endpoints honor "no social features": the
/// only multi-user surface is administration, never cross-user data access.
fn require_admin(user: &AuthUser) -> Result<(), SyncError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(SyncError::Forbidden("admin role required".into()))
    }
}

/// `GET /api/v1/admin/users` — list all accounts (admin only).
///
/// Returns only non-sensitive fields; SRP verifiers and wrapped key material are
/// never exposed.
pub async fn list_users(
    State(db_path): State<PathBuf>,
    user: AuthUser,
) -> Result<Json<UserListResponse>, SyncError> {
    require_admin(&user)?;

    let users = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<Vec<UserSummary>, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let mut stmt = db.conn.prepare(
                "SELECT id, email, role, status, created_at
                 FROM users ORDER BY created_at",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(UserSummary {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        role: row.get(2)?,
                        status: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(UserListResponse { users }))
}

/// `POST /api/v1/admin/users/{id}/status` — enable/disable a user (admin only).
///
/// Guards: an admin cannot disable their own account, and the last remaining
/// admin cannot be disabled (so the instance always has at least one admin).
/// Disabling a user invalidates their active sessions.
pub async fn set_user_status(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Path(target_id): Path<String>,
    Json(req): Json<SetUserStatusRequest>,
) -> Result<Json<UserSummary>, SyncError> {
    require_admin(&user)?;

    let new_status = match req.status.as_str() {
        "active" | "disabled" => req.status.clone(),
        _ => {
            return Err(SyncError::BadRequest(
                "status must be 'active' or 'disabled'".into(),
            ));
        }
    };

    if new_status == "disabled" && target_id == user.user_id {
        return Err(SyncError::Forbidden(
            "you cannot disable your own account".into(),
        ));
    }

    let summary = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let target_id = target_id.clone();
        move || -> Result<UserSummary, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Load the target (404 if absent).
            let (email, role, current_status, created_at): (String, String, String, String) = db
                .conn
                .query_row(
                    "SELECT email, role, status, created_at FROM users WHERE id = ?1",
                    [&target_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| SyncError::NotFound(format!("user {target_id} not found")))?;

            // Don't disable the last active admin.
            if new_status == "disabled" && role == "admin" {
                let other_active_admins: i64 = db.conn.query_row(
                    "SELECT COUNT(*) FROM users
                     WHERE role = 'admin' AND status = 'active' AND id != ?1",
                    [&target_id],
                    |row| row.get(0),
                )?;
                if other_active_admins == 0 {
                    return Err(SyncError::Forbidden(
                        "cannot disable the last remaining admin".into(),
                    ));
                }
            }

            if new_status != current_status {
                db.conn.execute(
                    "UPDATE users SET status = ?1 WHERE id = ?2",
                    rusqlite::params![new_status, target_id],
                )?;
                // Revoke active sessions when disabling.
                if new_status == "disabled" {
                    db.conn
                        .execute("DELETE FROM user_sessions WHERE user_id = ?1", [&target_id])?;
                }
            }

            Ok(UserSummary {
                id: target_id,
                email,
                role,
                status: new_status,
                created_at,
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(summary))
}

/// `GET /api/v1/admin/registration` — read the open-registration flag (admin only).
pub async fn get_registration(
    State(db_path): State<PathBuf>,
    user: AuthUser,
) -> Result<Json<RegistrationConfigResponse>, SyncError> {
    require_admin(&user)?;

    let open = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<bool, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let flag: i64 = db
                .conn
                .query_row(
                    "SELECT registration_open FROM instance_config WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            Ok(flag != 0)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(RegistrationConfigResponse {
        registration_open: open,
    }))
}

/// `PUT /api/v1/admin/registration` — open/close self-registration (admin only).
pub async fn set_registration(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Json(req): Json<SetRegistrationRequest>,
) -> Result<Json<RegistrationConfigResponse>, SyncError> {
    require_admin(&user)?;

    let open = req.open;
    tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<(), SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            db.conn.execute(
                "UPDATE instance_config
                 SET registration_open = ?1, updated_at = datetime('now')
                 WHERE id = 1",
                [i64::from(open)],
            )?;
            Ok(())
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(RegistrationConfigResponse {
        registration_open: open,
    }))
}

/// `GET /api/v1/admin/device-approvals` — read the device-approval gate (admin only).
pub async fn get_device_approvals(
    State(db_path): State<PathBuf>,
    user: AuthUser,
) -> Result<Json<DeviceApprovalsConfigResponse>, SyncError> {
    require_admin(&user)?;

    let required = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<bool, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            let flag: i64 = db
                .conn
                .query_row(
                    "SELECT device_approvals_required FROM instance_config WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            Ok(flag != 0)
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(DeviceApprovalsConfigResponse {
        device_approvals_required: required,
    }))
}

/// `PUT /api/v1/admin/device-approvals` — enable/disable the device-approval
/// gate (admin only). When enabled, a newly enrolled device joining a library
/// that already has an active device is held `pending` until approved.
pub async fn set_device_approvals(
    State(db_path): State<PathBuf>,
    user: AuthUser,
    Json(req): Json<SetDeviceApprovalsRequest>,
) -> Result<Json<DeviceApprovalsConfigResponse>, SyncError> {
    require_admin(&user)?;

    let required = req.required;
    tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> Result<(), SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;
            db.conn.execute(
                "UPDATE instance_config
                 SET device_approvals_required = ?1, updated_at = datetime('now')
                 WHERE id = 1",
                [i64::from(required)],
            )?;
            Ok(())
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(DeviceApprovalsConfigResponse {
        device_approvals_required: required,
    }))
}
