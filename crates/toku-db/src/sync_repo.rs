use std::str::FromStr;

use rusqlite::{OptionalExtension, params};
use toku_core::{DeviceIdentity, EntityType, HlcTimestamp, HybridClock, OpType, SyncOp};
use uuid::Uuid;

use crate::{Database, DbError};

/// Which side of a sync conflict to keep when resolving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKeep {
    /// Keep this device's local value.
    Local,
    /// Keep the incoming remote value.
    Remote,
}

/// A stored sync conflict awaiting user review.
///
/// Mirrors a row of the `sync_conflicts` table. Conflicts are only produced for
/// note and review edits that collide across devices (all other entity types
/// merge silently).
#[derive(Debug, Clone)]
pub struct SyncConflict {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub field_name: Option<String>,
    pub local_value: Option<String>,
    pub remote_value: Option<String>,
    pub local_hlc: String,
    pub remote_hlc: String,
    pub resolved: bool,
    pub resolved_at: Option<String>,
    pub created_at: String,
}

impl SyncConflict {
    /// The value that would remain if the given side is kept.
    pub fn kept_value(&self, keep: ConflictKeep) -> Option<&str> {
        match keep {
            ConflictKeep::Local => self.local_value.as_deref(),
            ConflictKeep::Remote => self.remote_value.as_deref(),
        }
    }
}

/// Sync persistence operations: ops, cursors, and device identity.
pub struct SyncRepository<'a> {
    db: &'a Database,
}

impl<'a> SyncRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert a sync op into the local staging table.
    pub fn insert_op(&self, op: &SyncOp) -> Result<(), DbError> {
        let fields_json = op.fields.as_ref().map(|v| v.to_string());
        self.db.conn.execute(
            "INSERT INTO sync_ops (op_id, device_id, hlc, entity_type, entity_id,
             op_type, fields_json, checksum, pushed_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
            params![
                op.op_id.to_string(),
                op.device_id.to_string(),
                op.hlc.to_canonical(),
                op.entity_type.as_str(),
                op.entity_id.to_string(),
                op.op_type.as_str(),
                fields_json,
                op.checksum,
                op.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Emit a local sync op for a frontend mutation and stage it for push.
    ///
    /// This is the single choke-point through which every domain mutation
    /// records its op. It is a **no-op when no device identity is configured**,
    /// preserving the offline-first guarantee: op creation never requires sync
    /// setup or network access.
    ///
    /// The op is stored in plaintext — `fields` are encrypted later, at push
    /// time (see `toku-sync-client`). The HLC is seeded past the highest local
    /// HLC so emitted ops are monotonic. For book fields, local provenance is
    /// recorded (mirroring the merge engine) so that a later pull of a staler
    /// remote edit cannot clobber this local write.
    ///
    /// Errors propagate to the caller so a failed op rolls back the enclosing
    /// transaction: sync correctness must not be silently dropped.
    pub fn emit_local_op(
        &self,
        entity_type: EntityType,
        entity_id: Uuid,
        op_type: OpType,
        fields: Option<serde_json::Value>,
    ) -> Result<(), DbError> {
        let Some(identity) = self.get_device()? else {
            return Ok(());
        };

        let hlc = self.next_local_hlc(&identity.device_id)?;

        // Record local field provenance for book create/update writes so that
        // subsequent merges (pulled remote edits) respect this write's recency.
        // Session/Progress/Tag entities are append-only or immutable and carry
        // no per-field HLC bookkeeping, so they need no provenance here.
        if entity_type == EntityType::Book
            && matches!(op_type, OpType::Create | OpType::Update)
            && let Some(obj) = fields.as_ref().and_then(|f| f.as_object())
        {
            let hlc_str = hlc.to_canonical();
            let book_id = entity_id.to_string();
            for field_name in obj.keys() {
                self.upsert_book_provenance(&book_id, field_name, &hlc_str)?;
            }
        }

        let op = SyncOp::new(
            identity.device_id,
            hlc,
            entity_type,
            entity_id,
            op_type,
            fields,
        );
        self.insert_op(&op)
    }

    /// Build the next monotonic local HLC, seeded past the highest HLC already
    /// present in the local op log (across all devices). This guarantees each
    /// emitted op sorts after every existing one, avoiding same-millisecond
    /// collisions that could otherwise drop a rapid second edit on the peer.
    fn next_local_hlc(&self, device_id: &Uuid) -> Result<HlcTimestamp, DbError> {
        let mut clock = HybridClock::new(device_id);
        let max_hlc: Option<String> = self
            .db
            .conn
            .query_row("SELECT MAX(hlc) FROM sync_ops", [], |row| row.get(0))
            .optional()?
            .flatten();

        if let Some(max) = max_hlc
            && let Ok(remote) = HlcTimestamp::from_str(&max)
        {
            return Ok(clock.update(&remote));
        }
        Ok(clock.now())
    }

    /// Upsert book field provenance with the given sync HLC. Mirrors the merge
    /// engine's provenance write so local and remote edits share identical LWW
    /// bookkeeping. On conflict only `sync_hlc` advances, leaving any existing
    /// metadata `source` (e.g. from an importer) untouched.
    fn upsert_book_provenance(
        &self,
        book_id: &str,
        field_name: &str,
        hlc: &str,
    ) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES (?1, ?2, 'sync', ?3, 0, ?3)
             ON CONFLICT (book_id, field_name) DO UPDATE SET sync_hlc = ?3
             WHERE sync_hlc IS NULL OR sync_hlc < ?3",
            params![book_id, field_name, hlc],
        )?;
        Ok(())
    }

    /// Get all ops that have not been pushed yet, ordered by HLC.
    pub fn get_unpushed_ops(&self) -> Result<Vec<SyncOp>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT op_id, device_id, hlc, entity_type, entity_id,
                    op_type, fields_json, checksum, created_at
             FROM sync_ops
             WHERE pushed_at IS NULL
             ORDER BY hlc ASC",
        )?;

        let ops = stmt
            .query_map([], |row| {
                let op_id_str: String = row.get(0)?;
                let device_id_str: String = row.get(1)?;
                let hlc_str: String = row.get(2)?;
                let entity_type_str: String = row.get(3)?;
                let entity_id_str: String = row.get(4)?;
                let op_type_str: String = row.get(5)?;
                let fields_json: Option<String> = row.get(6)?;
                let checksum: String = row.get(7)?;
                let created_at_str: String = row.get(8)?;

                Ok(SyncOpRow {
                    op_id_str,
                    device_id_str,
                    hlc_str,
                    entity_type_str,
                    entity_id_str,
                    op_type_str,
                    fields_json,
                    checksum,
                    created_at_str,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        ops.into_iter().map(|r| r.into_sync_op()).collect()
    }

    /// Mark the given ops as pushed (set pushed_at to now).
    pub fn mark_ops_pushed(&self, op_ids: &[Uuid]) -> Result<u64, DbError> {
        if op_ids.is_empty() {
            return Ok(0);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let placeholders: Vec<String> = op_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "UPDATE sync_ops SET pushed_at = ?1 WHERE op_id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = self.db.conn.prepare(&sql)?;
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        for id in op_ids {
            param_values.push(Box::new(id.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();
        let count = stmt.execute(refs.as_slice())?;
        Ok(count as u64)
    }

    /// Get the current device identity, if one has been set.
    pub fn get_device(&self) -> Result<Option<DeviceIdentity>, DbError> {
        let result = self
            .db
            .conn
            .query_row(
                "SELECT device_id, device_name, created_at FROM sync_device LIMIT 1",
                [],
                |row| {
                    let id_str: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let created_str: String = row.get(2)?;
                    Ok((id_str, name, created_str))
                },
            )
            .optional()?;

        match result {
            Some((id_str, name, created_str)) => {
                let device_id = Uuid::parse_str(&id_str)
                    .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
                Ok(Some(DeviceIdentity {
                    device_id,
                    device_name: name,
                    created_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get or create the device identity. If no device exists, create one
    /// with the given name.
    pub fn get_or_create_device(&self, name: &str) -> Result<DeviceIdentity, DbError> {
        if let Some(existing) = self.get_device()? {
            return Ok(existing);
        }

        let device = DeviceIdentity::new(name);
        self.db.conn.execute(
            "INSERT INTO sync_device (device_id, device_name, created_at) VALUES (?1, ?2, ?3)",
            params![
                device.device_id.to_string(),
                device.device_name,
                device.created_at.to_rfc3339(),
            ],
        )?;
        Ok(device)
    }

    /// Get or create the device identity using an explicit `device_id`.
    ///
    /// Used during sync init so the local device identity matches the
    /// server-assigned device id. This keeps emitted ops (which carry this
    /// id) consistent with the server's "exclude own device" pull filter,
    /// preventing a device from pulling back its own ops.
    pub fn get_or_create_device_with_id(
        &self,
        device_id: Uuid,
        name: &str,
    ) -> Result<DeviceIdentity, DbError> {
        if let Some(existing) = self.get_device()? {
            return Ok(existing);
        }

        let device = DeviceIdentity {
            device_id,
            device_name: name.to_string(),
            created_at: chrono::Utc::now(),
        };
        self.db.conn.execute(
            "INSERT INTO sync_device (device_id, device_name, created_at) VALUES (?1, ?2, ?3)",
            params![
                device.device_id.to_string(),
                device.device_name,
                device.created_at.to_rfc3339(),
            ],
        )?;
        Ok(device)
    }

    /// Get a sync cursor value by key ("push_cursor" or "pull_cursor").
    pub fn get_cursor(&self, key: &str) -> Result<Option<String>, DbError> {
        let result = self
            .db
            .conn
            .query_row(
                "SELECT value FROM sync_cursors WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Set a sync cursor value (upsert).
    pub fn set_cursor(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT INTO sync_cursors (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Clear a sync cursor so the next sync starts from scratch. Used by
    /// `toku sync bootstrap --reset-cursor` to force a full re-pull.
    pub fn clear_cursor(&self, key: &str) -> Result<(), DbError> {
        self.db
            .conn
            .execute("DELETE FROM sync_cursors WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Record that this device has completed its initial sync bootstrap
    /// (backfill on first opt-in, or restore on new-device enroll / deferred
    /// login). Device-local; never synced. See ADR-013 (D3).
    pub fn mark_bootstrapped(&self) -> Result<(), DbError> {
        self.db.conn.execute(
            "UPDATE sync_device SET bootstrapped_at = ?1 WHERE bootstrapped_at IS NULL",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Whether this device has already completed its initial sync bootstrap.
    /// Returns `false` when no device identity exists yet.
    pub fn is_bootstrapped(&self) -> Result<bool, DbError> {
        let result: Option<Option<String>> = self
            .db
            .conn
            .query_row(
                "SELECT bootstrapped_at FROM sync_device LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(matches!(result, Some(Some(_))))
    }

    /// Insert a sync op received from the server.
    ///
    /// Remote ops are stored with `pushed_at` set to now (they don't need
    /// to be pushed back — they came from the server). Duplicates are
    /// silently ignored.
    pub fn insert_remote_op(&self, op: &SyncOp) -> Result<(), DbError> {
        let fields_json = op.fields.as_ref().map(|v| v.to_string());
        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "INSERT OR IGNORE INTO sync_ops (op_id, device_id, hlc, entity_type, entity_id,
             op_type, fields_json, checksum, pushed_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                op.op_id.to_string(),
                op.device_id.to_string(),
                op.hlc.to_canonical(),
                op.entity_type.as_str(),
                op.entity_id.to_string(),
                op.op_type.as_str(),
                fields_json,
                op.checksum,
                now,
                op.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Count the number of unpushed ops.
    pub fn count_unpushed_ops(&self) -> Result<i64, DbError> {
        let count: i64 = self.db.conn.query_row(
            "SELECT COUNT(*) FROM sync_ops WHERE pushed_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Conflicts
    // -----------------------------------------------------------------------

    /// List all unresolved sync conflicts, oldest first.
    pub fn list_unresolved_conflicts(&self) -> Result<Vec<SyncConflict>, DbError> {
        let mut stmt = self.db.conn.prepare(
            "SELECT id, entity_type, entity_id, field_name, local_value, remote_value,
                    local_hlc, remote_hlc, resolved, resolved_at, created_at
             FROM sync_conflicts
             WHERE resolved = 0
             ORDER BY created_at ASC",
        )?;
        let conflicts = stmt
            .query_map([], Self::map_conflict_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(conflicts)
    }

    /// Fetch a single conflict by id.
    pub fn get_conflict(&self, id: &str) -> Result<Option<SyncConflict>, DbError> {
        let conflict = self
            .db
            .conn
            .query_row(
                "SELECT id, entity_type, entity_id, field_name, local_value, remote_value,
                        local_hlc, remote_hlc, resolved, resolved_at, created_at
                 FROM sync_conflicts WHERE id = ?1",
                params![id],
                Self::map_conflict_row,
            )
            .optional()?;
        Ok(conflict)
    }

    /// Count unresolved conflicts.
    pub fn count_unresolved_conflicts(&self) -> Result<i64, DbError> {
        let count: i64 = self.db.conn.query_row(
            "SELECT COUNT(*) FROM sync_conflicts WHERE resolved = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Resolve a single conflict by keeping the local or remote value.
    ///
    /// Applies the chosen value to the underlying note/review entity, bumps the
    /// per-field HLC, emits a new sync op so the resolution propagates to other
    /// devices, and marks the conflict resolved. A no-op (returns `false`) if the
    /// conflict is missing or already resolved.
    pub fn resolve_conflict(&self, id: &str, keep: ConflictKeep) -> Result<bool, DbError> {
        let Some(conflict) = self.get_conflict(id)? else {
            return Ok(false);
        };
        if conflict.resolved {
            return Ok(false);
        }

        self.apply_resolution(&conflict, conflict.kept_value(keep))?;

        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "UPDATE sync_conflicts SET resolved = 1, resolved_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(true)
    }

    /// Resolve a single conflict with a user-supplied merged value.
    ///
    /// Unlike [`resolve_conflict`], this writes an arbitrary value (rather than
    /// the local or remote side) to the underlying note/review entity, bumps the
    /// per-field HLC, emits a propagating sync op, and marks the conflict
    /// resolved. A no-op (returns `false`) if the conflict is missing or already
    /// resolved. Returns [`DbError::InvalidOperation`] if the value is invalid
    /// for the field (e.g. a non-integer or out-of-range review rating).
    pub fn resolve_conflict_with_value(
        &self,
        id: &str,
        value: Option<&str>,
    ) -> Result<bool, DbError> {
        let Some(conflict) = self.get_conflict(id)? else {
            return Ok(false);
        };
        if conflict.resolved {
            return Ok(false);
        }

        let field = conflict.field_name.as_deref().ok_or_else(|| {
            DbError::InvalidOperation("conflict has no field to resolve".to_string())
        })?;
        let entity_type = EntityType::from_str(&conflict.entity_type)
            .map_err(|_| DbError::InvalidOperation("unknown conflict entity type".to_string()))?;

        // Validate the merged value against the target field's domain.
        if let (EntityType::Review, "rating") = (entity_type, field)
            && let Some(v) = value
        {
            let rating: i64 = v
                .trim()
                .parse()
                .map_err(|_| DbError::InvalidOperation(format!("invalid rating value: {v:?}")))?;
            if !(0..=10).contains(&rating) {
                return Err(DbError::InvalidOperation(format!(
                    "rating must be between 0 and 10, got {rating}"
                )));
            }
        }

        self.apply_resolution(&conflict, value)?;

        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "UPDATE sync_conflicts SET resolved = 1, resolved_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(true)
    }

    /// Resolve every unresolved conflict with the same choice. Returns the count resolved.
    pub fn resolve_all_conflicts(&self, keep: ConflictKeep) -> Result<usize, DbError> {
        let conflicts = self.list_unresolved_conflicts()?;
        let mut resolved = 0;
        for conflict in &conflicts {
            self.apply_resolution(conflict, conflict.kept_value(keep))?;
            resolved += 1;
        }
        if resolved > 0 {
            let now = chrono::Utc::now().to_rfc3339();
            self.db.conn.execute(
                "UPDATE sync_conflicts SET resolved = 1, resolved_at = ?1 WHERE resolved = 0",
                params![now],
            )?;
        }
        Ok(resolved)
    }

    /// Write the given value into the live entity, bump its HLC, and emit a
    /// propagating sync op. Best-effort: if no device identity exists yet, the
    /// entity value is still updated but no op is generated.
    fn apply_resolution(
        &self,
        conflict: &SyncConflict,
        value: Option<&str>,
    ) -> Result<(), DbError> {
        let field = conflict.field_name.as_deref().ok_or_else(|| {
            DbError::InvalidOperation("conflict has no field to resolve".to_string())
        })?;

        let entity_type = EntityType::from_str(&conflict.entity_type)
            .map_err(|_| DbError::InvalidOperation("unknown conflict entity type".to_string()))?;
        let entity_uuid = Uuid::parse_str(&conflict.entity_id)
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;

        let device = self.get_device()?;
        let (hlc_canonical, device_id_str) = match &device {
            Some(identity) => {
                let mut clock = HybridClock::new(&identity.device_id);
                (clock.now().to_canonical(), identity.device_id.to_string())
            }
            None => (String::new(), String::new()),
        };

        let now = chrono::Utc::now().to_rfc3339();

        // Build the fields payload for the propagating op (typed per field).
        let field_value = match (entity_type, field) {
            (EntityType::Review, "rating") => match value {
                Some(v) => serde_json::json!(v.parse::<i64>().ok()),
                None => serde_json::Value::Null,
            },
            _ => match value {
                Some(v) => serde_json::json!(v),
                None => serde_json::Value::Null,
            },
        };

        match (entity_type, field) {
            (EntityType::Note, "content") => {
                self.db.conn.execute(
                    "UPDATE notes SET content = ?1, updated_at = ?2 WHERE id = ?3",
                    params![value.unwrap_or(""), now, conflict.entity_id],
                )?;
            }
            (EntityType::Review, "content") => {
                self.db.conn.execute(
                    "UPDATE reviews SET content = ?1, updated_at = ?2 WHERE id = ?3",
                    params![value, now, conflict.entity_id],
                )?;
            }
            (EntityType::Review, "rating") => {
                let rating: Option<i64> = value.and_then(|v| v.parse().ok());
                self.db.conn.execute(
                    "UPDATE reviews SET rating = ?1, updated_at = ?2 WHERE id = ?3",
                    params![rating, now, conflict.entity_id],
                )?;
            }
            _ => {
                return Err(DbError::InvalidOperation(format!(
                    "cannot resolve conflict for {}.{field}",
                    conflict.entity_type
                )));
            }
        }

        if let Some(identity) = device {
            self.db.conn.execute(
                "INSERT INTO sync_entity_hlc (entity_type, entity_id, field_name, sync_hlc, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (entity_type, entity_id, field_name)
                 DO UPDATE SET sync_hlc = ?4, device_id = ?5",
                params![
                    conflict.entity_type,
                    conflict.entity_id,
                    field,
                    hlc_canonical,
                    device_id_str
                ],
            )?;

            let hlc = HlcTimestamp::from_str(&hlc_canonical)
                .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
            let op = SyncOp::new(
                identity.device_id,
                hlc,
                entity_type,
                entity_uuid,
                OpType::Update,
                Some(serde_json::json!({ field: field_value })),
            );
            self.insert_op(&op)?;
        }

        Ok(())
    }

    fn map_conflict_row(row: &rusqlite::Row) -> rusqlite::Result<SyncConflict> {
        Ok(SyncConflict {
            id: row.get(0)?,
            entity_type: row.get(1)?,
            entity_id: row.get(2)?,
            field_name: row.get(3)?,
            local_value: row.get(4)?,
            remote_value: row.get(5)?,
            local_hlc: row.get(6)?,
            remote_hlc: row.get(7)?,
            resolved: row.get::<_, i64>(8)? != 0,
            resolved_at: row.get(9)?,
            created_at: row.get(10)?,
        })
    }
}

/// Internal helper for deserializing rows before parsing UUIDs/HLC.
struct SyncOpRow {
    op_id_str: String,
    device_id_str: String,
    hlc_str: String,
    entity_type_str: String,
    entity_id_str: String,
    op_type_str: String,
    fields_json: Option<String>,
    checksum: String,
    created_at_str: String,
}

impl SyncOpRow {
    fn into_sync_op(self) -> Result<SyncOp, DbError> {
        let op_id = Uuid::parse_str(&self.op_id_str)
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
        let device_id = Uuid::parse_str(&self.device_id_str)
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
        let hlc: HlcTimestamp = self
            .hlc_str
            .parse()
            .map_err(|e: toku_core::TokuError| DbError::InvalidOperation(e.to_string()))?;
        let entity_type: EntityType = self
            .entity_type_str
            .parse()
            .map_err(|e: toku_core::TokuError| DbError::InvalidOperation(e.to_string()))?;
        let entity_id = Uuid::parse_str(&self.entity_id_str)
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
        let op_type: OpType = self
            .op_type_str
            .parse()
            .map_err(|e: toku_core::TokuError| DbError::InvalidOperation(e.to_string()))?;
        let fields = self
            .fields_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .map_err(|e| DbError::InvalidOperation(e.to_string()))?;

        Ok(SyncOp {
            v: 1,
            op_id,
            device_id,
            hlc,
            entity_type,
            entity_id,
            op_type,
            fields,
            encrypted: None,
            checksum: self.checksum,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toku_core::HybridClock;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn test_device() -> Uuid {
        Uuid::parse_str("01972123-abcd-7000-8000-000000000001").unwrap()
    }

    fn make_op(clock: &mut HybridClock, entity_type: EntityType, op_type: OpType) -> SyncOp {
        let hlc = clock.now();
        SyncOp::new(
            test_device(),
            hlc,
            entity_type,
            Uuid::now_v7(),
            op_type,
            Some(serde_json::json!({"title": "Dune"})),
        )
    }

    #[test]
    fn insert_and_query_unpushed() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let mut clock = HybridClock::new(&test_device());

        let op = make_op(&mut clock, EntityType::Book, OpType::Create);
        repo.insert_op(&op).unwrap();

        let unpushed = repo.get_unpushed_ops().unwrap();
        assert_eq!(unpushed.len(), 1);
        assert_eq!(unpushed[0].op_id, op.op_id);
        assert_eq!(unpushed[0].entity_type, EntityType::Book);
        assert_eq!(unpushed[0].op_type, OpType::Create);
        assert!(unpushed[0].verify_checksum());
    }

    #[test]
    fn mark_ops_pushed_excludes_from_unpushed() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let mut clock = HybridClock::new(&test_device());

        let op1 = make_op(&mut clock, EntityType::Book, OpType::Create);
        let op2 = make_op(&mut clock, EntityType::Book, OpType::Update);
        repo.insert_op(&op1).unwrap();
        repo.insert_op(&op2).unwrap();

        assert_eq!(repo.count_unpushed_ops().unwrap(), 2);

        let marked = repo.mark_ops_pushed(&[op1.op_id]).unwrap();
        assert_eq!(marked, 1);

        let unpushed = repo.get_unpushed_ops().unwrap();
        assert_eq!(unpushed.len(), 1);
        assert_eq!(unpushed[0].op_id, op2.op_id);
    }

    #[test]
    fn mark_ops_pushed_empty_ids() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let marked = repo.mark_ops_pushed(&[]).unwrap();
        assert_eq!(marked, 0);
    }

    #[test]
    fn device_identity_get_or_create() {
        let db = test_db();
        let repo = SyncRepository::new(&db);

        assert!(repo.get_device().unwrap().is_none());

        let dev = repo.get_or_create_device("Test Laptop").unwrap();
        assert_eq!(dev.device_name, "Test Laptop");

        // Second call returns the same device
        let dev2 = repo.get_or_create_device("Different Name").unwrap();
        assert_eq!(dev2.device_id, dev.device_id);
        assert_eq!(dev2.device_name, "Test Laptop"); // original name preserved
    }

    #[test]
    fn cursor_get_set_roundtrip() {
        let db = test_db();
        let repo = SyncRepository::new(&db);

        assert!(repo.get_cursor("push_cursor").unwrap().is_none());

        repo.set_cursor("push_cursor", "op-123").unwrap();
        assert_eq!(
            repo.get_cursor("push_cursor").unwrap().as_deref(),
            Some("op-123")
        );

        // Upsert overwrites
        repo.set_cursor("push_cursor", "op-456").unwrap();
        assert_eq!(
            repo.get_cursor("push_cursor").unwrap().as_deref(),
            Some("op-456")
        );
    }

    #[test]
    fn unpushed_ops_ordered_by_hlc() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let mut clock = HybridClock::new(&test_device());

        let op1 = make_op(&mut clock, EntityType::Book, OpType::Create);
        let op2 = make_op(&mut clock, EntityType::Session, OpType::Create);
        let op3 = make_op(&mut clock, EntityType::Book, OpType::Update);

        // Insert out of order
        repo.insert_op(&op3).unwrap();
        repo.insert_op(&op1).unwrap();
        repo.insert_op(&op2).unwrap();

        let unpushed = repo.get_unpushed_ops().unwrap();
        assert_eq!(unpushed.len(), 3);
        // Verify HLC ordering
        assert!(unpushed[0].hlc < unpushed[1].hlc);
        assert!(unpushed[1].hlc < unpushed[2].hlc);
    }

    #[test]
    fn sync_op_delete_no_fields() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let mut clock = HybridClock::new(&test_device());

        let hlc = clock.now();
        let op = SyncOp::new(
            test_device(),
            hlc,
            EntityType::Book,
            Uuid::now_v7(),
            OpType::Delete,
            None,
        );
        repo.insert_op(&op).unwrap();

        let unpushed = repo.get_unpushed_ops().unwrap();
        assert_eq!(unpushed.len(), 1);
        assert!(unpushed[0].fields.is_none());
        assert!(unpushed[0].verify_checksum());
    }

    #[test]
    fn migration_is_backward_compatible() {
        // Opening a new in-memory DB runs all migrations including V12.
        // If any migration fails, open_in_memory() would return an error.
        let db = test_db();
        // Verify sync tables exist
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sync_ops", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sync_cursors", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sync_device", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // ── Conflict tests ──────────────────────────────────────────────

    fn seed_book_and_note(db: &Database, note_id: &str, content: &str) {
        let now = "2026-06-15T10:00:00Z";
        db.conn
            .execute(
                "INSERT INTO books (id, title, created_at, updated_at)
                 VALUES (?1, 'Dune', ?2, ?2)",
                params!["01972123-aaaa-7000-8000-000000000abc", now],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO notes (id, book_id, content, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![
                    note_id,
                    "01972123-aaaa-7000-8000-000000000abc",
                    content,
                    now
                ],
            )
            .unwrap();
    }

    fn insert_conflict(
        db: &Database,
        id: &str,
        entity_id: &str,
        local: &str,
        remote: &str,
        resolved: bool,
    ) {
        db.conn
            .execute(
                "INSERT INTO sync_conflicts
                 (id, entity_type, entity_id, field_name, local_value, remote_value,
                  local_hlc, remote_hlc, resolved, created_at)
                 VALUES (?1, 'note', ?2, 'content', ?3, ?4, ?5, ?6, ?7,
                         '2026-06-15T10:30:00Z')",
                params![
                    id,
                    entity_id,
                    local,
                    remote,
                    format!("hlc-a-{id}"),
                    format!("hlc-b-{id}"),
                    resolved as i64
                ],
            )
            .unwrap();
    }

    #[test]
    fn list_and_count_unresolved_conflicts() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "remote text");

        insert_conflict(&db, "c1", &note_id, "local text", "remote text", false);
        insert_conflict(&db, "c2", &note_id, "old local", "old remote", true);

        let unresolved = repo.list_unresolved_conflicts().unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].id, "c1");
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 1);
    }

    #[test]
    fn resolve_conflict_keep_local_updates_note_and_emits_op() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "remote text");
        insert_conflict(&db, "c1", &note_id, "local text", "remote text", false);

        let resolved = repo.resolve_conflict("c1", ConflictKeep::Local).unwrap();
        assert!(resolved);

        // Note content reverted to the kept local value.
        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "local text");

        // Conflict marked resolved and excluded from listings.
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 0);
        let conflict = repo.get_conflict("c1").unwrap().unwrap();
        assert!(conflict.resolved);
        assert!(conflict.resolved_at.is_some());

        // A propagating op was emitted.
        let ops = repo.get_unpushed_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].entity_type, EntityType::Note);
        assert_eq!(ops[0].op_type, OpType::Update);
    }

    #[test]
    fn resolve_conflict_keep_remote_keeps_remote_value() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "remote text");
        insert_conflict(&db, "c1", &note_id, "local text", "remote text", false);

        assert!(repo.resolve_conflict("c1", ConflictKeep::Remote).unwrap());

        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "remote text");
    }

    #[test]
    fn resolve_all_conflicts_resolves_every_unresolved() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "remote text");
        insert_conflict(&db, "c1", &note_id, "local 1", "remote 1", false);
        insert_conflict(&db, "c2", &note_id, "local 2", "remote 2", false);

        let count = repo.resolve_all_conflicts(ConflictKeep::Local).unwrap();
        assert_eq!(count, 2);
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 0);
    }

    #[test]
    fn resolve_missing_or_resolved_conflict_is_noop() {
        let db = test_db();
        let repo = SyncRepository::new(&db);

        assert!(
            !repo
                .resolve_conflict("missing", ConflictKeep::Local)
                .unwrap()
        );

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "x");
        insert_conflict(&db, "c1", &note_id, "a", "b", true);
        assert!(!repo.resolve_conflict("c1", ConflictKeep::Local).unwrap());
    }

    fn seed_review(db: &Database, review_id: &str, content: Option<&str>, rating: Option<i64>) {
        let now = "2026-06-15T10:00:00Z";
        db.conn
            .execute(
                "INSERT OR IGNORE INTO books (id, title, created_at, updated_at)
                 VALUES (?1, 'Dune', ?2, ?2)",
                params!["01972123-aaaa-7000-8000-000000000abc", now],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO reviews (id, book_id, content, rating, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    review_id,
                    "01972123-aaaa-7000-8000-000000000abc",
                    content,
                    rating,
                    now
                ],
            )
            .unwrap();
    }

    fn insert_rating_conflict(db: &Database, id: &str, entity_id: &str, local: &str, remote: &str) {
        db.conn
            .execute(
                "INSERT INTO sync_conflicts
                 (id, entity_type, entity_id, field_name, local_value, remote_value,
                  local_hlc, remote_hlc, resolved, created_at)
                 VALUES (?1, 'review', ?2, 'rating', ?3, ?4, ?5, ?6, 0,
                         '2026-06-15T10:30:00Z')",
                params![
                    id,
                    entity_id,
                    local,
                    remote,
                    format!("hlc-a-{id}"),
                    format!("hlc-b-{id}")
                ],
            )
            .unwrap();
    }

    #[test]
    fn resolve_conflict_with_value_updates_note_and_emits_op() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "remote text");
        insert_conflict(&db, "c1", &note_id, "local text", "remote text", false);

        let resolved = repo
            .resolve_conflict_with_value("c1", Some("merged text"))
            .unwrap();
        assert!(resolved);

        // Note content set to the custom merged value.
        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "merged text");

        // Conflict marked resolved and excluded from listings.
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 0);
        let conflict = repo.get_conflict("c1").unwrap().unwrap();
        assert!(conflict.resolved);
        assert!(conflict.resolved_at.is_some());

        // A propagating op was emitted.
        let ops = repo.get_unpushed_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].entity_type, EntityType::Note);
        assert_eq!(ops[0].op_type, OpType::Update);
    }

    #[test]
    fn resolve_conflict_with_value_updates_review_rating() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let review_id = Uuid::now_v7().to_string();
        seed_review(&db, &review_id, Some("nice"), Some(4));
        insert_rating_conflict(&db, "c1", &review_id, "4", "8");

        assert!(repo.resolve_conflict_with_value("c1", Some("6")).unwrap());

        let rating: i64 = db
            .conn
            .query_row(
                "SELECT rating FROM reviews WHERE id = ?1",
                params![review_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rating, 6);
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 0);
    }

    #[test]
    fn resolve_conflict_with_invalid_rating_is_rejected() {
        let db = test_db();
        let repo = SyncRepository::new(&db);
        repo.get_or_create_device("test-device").unwrap();

        let review_id = Uuid::now_v7().to_string();
        seed_review(&db, &review_id, Some("nice"), Some(4));
        insert_rating_conflict(&db, "c1", &review_id, "4", "8");

        assert!(repo.resolve_conflict_with_value("c1", Some("99")).is_err());
        assert!(
            repo.resolve_conflict_with_value("c1", Some("notnum"))
                .is_err()
        );
        // Conflict left unresolved after a rejected value.
        assert_eq!(repo.count_unresolved_conflicts().unwrap(), 1);
    }

    #[test]
    fn resolve_with_value_missing_or_resolved_conflict_is_noop() {
        let db = test_db();
        let repo = SyncRepository::new(&db);

        assert!(
            !repo
                .resolve_conflict_with_value("missing", Some("x"))
                .unwrap()
        );

        let note_id = Uuid::now_v7().to_string();
        seed_book_and_note(&db, &note_id, "x");
        insert_conflict(&db, "c1", &note_id, "a", "b", true);
        assert!(
            !repo
                .resolve_conflict_with_value("c1", Some("merged"))
                .unwrap()
        );
    }
}
