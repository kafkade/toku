use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::auth::{AuthDevice, sha256_hex};
use crate::db::SyncDatabase;
use crate::error::SyncError;
use crate::models::{
    DeviceResponse, HealthResponse, OpPayload, PullQuery, PullResponse, PushRequest, PushResponse,
    RegisterRequest, RegisterResponse,
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
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, SyncError> {
    if req.device_name.is_empty() {
        return Err(SyncError::BadRequest("device_name is required".into()));
    }
    if req.library_id.is_empty() {
        return Err(SyncError::BadRequest("library_id is required".into()));
    }

    let token = generate_token();
    let token_hash = sha256_hex(&token);
    let device_id = uuid::Uuid::now_v7().to_string();

    let resp = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        let library_id = req.library_id.clone();
        let device_name = req.device_name.clone();
        let device_id = device_id.clone();
        let token_hash = token_hash.clone();
        move || -> Result<RegisterResponse, SyncError> {
            let db = SyncDatabase::open_no_migrate(&db_path)?;

            // Auto-create library if it doesn't exist
            db.conn.execute(
                "INSERT OR IGNORE INTO libraries (id, created_at) VALUES (?1, datetime('now'))",
                [&library_id],
            )?;

            db.conn.execute(
                "INSERT INTO devices (device_id, library_id, device_name, auth_token_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![device_id, library_id, device_name, token_hash],
            )?;

            Ok(RegisterResponse {
                device_id,
                library_id,
                auth_token: String::new(), // filled in below
            })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(RegisterResponse {
        auth_token: token,
        ..resp
    }))
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
            if let Some(last_op) = ops.last() {
                tx.execute(
                    "INSERT INTO cursors (device_id, cursor_type, op_id, updated_at)
                     VALUES (?1, 'push', ?2, datetime('now'))
                     ON CONFLICT (device_id, cursor_type) DO UPDATE SET
                       op_id = excluded.op_id,
                       updated_at = excluded.updated_at",
                    rusqlite::params![device_id, last_op.op_id],
                )?;
            }

            tx.commit()?;

            Ok(PushResponse {
                accepted,
                duplicates,
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

            let ops: Vec<OpPayload> = if let Some(ref since_op_id) = since {
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
                             WHERE library_id = ?1 AND (hlc > ?2 OR (hlc = ?2 AND op_id > ?3))
                             ORDER BY hlc, op_id
                             LIMIT ?4",
                        )?;
                        stmt.query_map(
                            rusqlite::params![library_id, hlc, since_op_id, MAX_BATCH_SIZE_SQL],
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
                // No cursor — return all ops from the beginning
                let mut stmt = db.conn.prepare(
                    "SELECT op_id, device_id, hlc, entity_type, entity_id, op_type, payload
                     FROM ops
                     WHERE library_id = ?1
                     ORDER BY hlc, op_id
                     LIMIT ?2",
                )?;
                stmt.query_map(rusqlite::params![library_id, MAX_BATCH_SIZE_SQL], row_to_op)?
                    .collect::<Result<Vec<_>, _>>()?
            };

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

            Ok(PullResponse { ops, cursor })
        }
    })
    .await
    .map_err(|e| SyncError::Internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

// ── Snapshot / Rekey stubs ──────────────────────────────────────────────────

pub async fn snapshot() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "snapshots are not yet implemented"
        })),
    )
}

pub async fn rekey() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "re-keying is not yet implemented"
        })),
    )
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

/// Generate a cryptographically random auth token (256-bit, base64url no-pad).
fn generate_token() -> String {
    use base64::Engine;
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
