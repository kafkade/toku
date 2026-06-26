use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::auth::{AuthDevice, generate_token, sha256_hex};
use crate::db::SyncDatabase;
use crate::error::SyncError;
use crate::models::{
    DeviceResponse, DownloadSnapshotResponse, HealthResponse, OpPayload, PullQuery, PullResponse,
    PushRequest, PushResponse, RegisterRequest, RegisterResponse, RekeyRequest, RekeyResponse,
    UploadSnapshotRequest, UploadSnapshotResponse,
};

const MAX_BATCH_SIZE: usize = 1000;
const MAX_BATCH_SIZE_SQL: i64 = MAX_BATCH_SIZE as i64;

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
                let session_token = bearer_token.ok_or(SyncError::Unauthorized)?;
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
