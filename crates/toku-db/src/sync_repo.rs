use rusqlite::{OptionalExtension, params};
use toku_core::{DeviceIdentity, EntityType, HlcTimestamp, OpType, SyncOp};
use uuid::Uuid;

use crate::{Database, DbError};

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

    /// Count the number of unpushed ops.
    pub fn count_unpushed_ops(&self) -> Result<i64, DbError> {
        let count: i64 = self.db.conn.query_row(
            "SELECT COUNT(*) FROM sync_ops WHERE pushed_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
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
}
