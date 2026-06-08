//! Entity-specific merge engine for applying remote sync ops to local state.
//!
//! Each entity type has its own merge strategy:
//! - **Book** — Last-write-wins per field, using HLC for ordering
//! - **Session** — Append-only (insert if new, never update/delete)
//! - **Progress** — Monotonic (only accept higher values)
//! - **Tag** — Apply add/remove in HLC order
//! - **Note/Review** — Not yet implemented (no dedicated tables)
//! - **Setting** — Not yet implemented (no syncable settings table)

use rusqlite::{OptionalExtension, params};
use toku_core::merge::{MergeConflict, MergeOutcome};
use toku_core::sync::{EntityType, OpType, SyncOp};
use uuid::Uuid;

use crate::{Database, DbError};

/// Allowed book fields for sync updates. Used as a whitelist to prevent
/// dynamic SQL injection — only these field names may appear in UPDATE
/// statements.
const BOOK_FIELDS: &[&str] = &[
    "title",
    "subtitle",
    "description",
    "page_count",
    "pub_date",
    "language",
    "format",
    "duration_minutes",
    "cover_hash",
    "work_id",
    "status",
    "rating",
];

/// Applies remote sync operations to the local database using
/// entity-specific merge strategies.
pub struct MergeEngine<'a> {
    db: &'a Database,
}

impl<'a> MergeEngine<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Apply a remote sync op to local state.
    ///
    /// Each entity type uses a different merge strategy. The operation is
    /// idempotent — applying the same op twice produces the same result.
    pub fn apply_op(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.entity_type {
            EntityType::Book => self.merge_book(op),
            EntityType::Session => self.merge_session(op),
            EntityType::Progress => self.merge_progress(op),
            EntityType::Tag => self.merge_tag(op),
            EntityType::Note => self.merge_note(op),
            EntityType::Review => self.merge_review(op),
            EntityType::Setting => self.merge_setting(op),
            EntityType::Device => Ok(MergeOutcome::Skipped {
                reason: "device ops handled separately",
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Book — Last-write-wins per field (HLC)
    // -----------------------------------------------------------------------

    fn merge_book(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_book_create(op),
            OpType::Update => self.merge_book_update(op),
            OpType::Delete => self.merge_book_delete(op),
        }
    }

    fn merge_book_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let book_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();

        // Check if book already exists
        let exists: bool = self.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE id = ?1)",
            params![book_id],
            |row| row.get(0),
        )?;

        if exists {
            // Book exists — treat as an update (field-level LWW)
            return self.merge_book_update(op);
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "book create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let subtitle = obj.get("subtitle").and_then(|v| v.as_str());
        let description = obj.get("description").and_then(|v| v.as_str());
        let page_count = obj.get("page_count").and_then(|v| v.as_i64());
        let pub_date = obj.get("pub_date").and_then(|v| v.as_str());
        let language = obj.get("language").and_then(|v| v.as_str());
        let format = obj
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("physical");
        let duration_minutes = obj.get("duration_minutes").and_then(|v| v.as_i64());
        let cover_hash = obj.get("cover_hash").and_then(|v| v.as_str());
        let work_id = obj.get("work_id").and_then(|v| v.as_str());
        let status = obj
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("want-to-read");
        let rating = obj.get("rating").and_then(|v| v.as_i64());
        let now = op.created_at.to_rfc3339();

        self.db.conn.execute(
            "INSERT INTO books (id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status, rating,
             created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                book_id,
                title,
                subtitle,
                description,
                page_count,
                pub_date,
                language,
                format,
                duration_minutes,
                cover_hash,
                work_id,
                status,
                rating,
                now,
                now,
            ],
        )?;

        // Record provenance for each field
        for field_name in BOOK_FIELDS {
            if obj.contains_key(*field_name) {
                self.upsert_provenance(&book_id, field_name, &hlc_str)?;
            }
        }

        Ok(MergeOutcome::Applied)
    }

    fn merge_book_update(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let book_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();

        // Check book exists
        let exists: bool = self.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE id = ?1)",
            params![book_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(MergeOutcome::Skipped {
                reason: "book not found",
            });
        }

        // Check if book is soft-deleted
        let deleted: bool = self.db.conn.query_row(
            "SELECT deleted_at IS NOT NULL FROM books WHERE id = ?1",
            params![book_id],
            |row| row.get(0),
        )?;
        if deleted {
            return Ok(MergeOutcome::Skipped {
                reason: "book is deleted",
            });
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Skipped {
                    reason: "update op has no fields",
                });
            }
        };
        let obj = fields.as_object().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let mut applied_any = false;

        for (field_name, value) in obj {
            if !BOOK_FIELDS.contains(&field_name.as_str()) {
                continue;
            }

            // Check sync_hlc in provenance — only apply if incoming is newer
            let local_hlc: Option<String> = self
                .db
                .conn
                .query_row(
                    "SELECT sync_hlc FROM metadata_provenance
                     WHERE book_id = ?1 AND field_name = ?2",
                    params![book_id, field_name],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();

            if let Some(ref local) = local_hlc
                && hlc_str <= *local
            {
                continue; // Local is newer or equal — skip this field
            }

            if field_name == "status"
                && let Some(new_status_str) = value.as_str()
            {
                let outcome = self.validate_status_transition(&book_id, new_status_str)?;
                if let Some(rejection) = outcome {
                    return Ok(MergeOutcome::Rejected { reason: rejection });
                }
            }

            // Apply the field update (field_name is whitelist-validated above)
            let sql = format!("UPDATE books SET {field_name} = ?1, updated_at = ?2 WHERE id = ?3");
            self.db
                .conn
                .execute(&sql, params![value_to_sql(value), now, book_id])?;

            self.upsert_provenance(&book_id, field_name, &hlc_str)?;
            applied_any = true;
        }

        if applied_any {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "all fields are up to date",
            })
        }
    }

    fn merge_book_delete(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let book_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();
        let device_id = op.device_id.to_string();

        let count = self.db.conn.execute(
            "UPDATE books SET deleted_at = ?1, deleted_by_device = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![hlc_str, device_id, book_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "book not found or already deleted",
            })
        }
    }

    /// Validate that a status transition is legal. Returns `Some(reason)` if
    /// the transition should be rejected, or `None` if it's valid.
    fn validate_status_transition(
        &self,
        book_id: &str,
        new_status_str: &str,
    ) -> Result<Option<String>, DbError> {
        use toku_core::ReadingStatus;

        let current_str: String = self.db.conn.query_row(
            "SELECT status FROM books WHERE id = ?1",
            params![book_id],
            |row| row.get(0),
        )?;

        let current: ReadingStatus = current_str
            .parse()
            .map_err(|e| DbError::InvalidOperation(format!("invalid current status: {e}")))?;
        let target: ReadingStatus = match new_status_str.parse() {
            Ok(s) => s,
            Err(e) => {
                return Ok(Some(format!(
                    "invalid target status '{new_status_str}': {e}"
                )));
            }
        };

        if current == target {
            return Ok(None); // Same status — no transition needed
        }

        if current.can_transition_to(&target) {
            Ok(None)
        } else {
            Ok(Some(format!(
                "invalid transition from {current} to {target}"
            )))
        }
    }

    // -----------------------------------------------------------------------
    // Session — Append-only
    // -----------------------------------------------------------------------

    fn merge_session(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_session_create(op),
            OpType::Update | OpType::Delete => Ok(MergeOutcome::Skipped {
                reason: "sessions are append-only",
            }),
        }
    }

    fn merge_session_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let session_id = op.entity_id.to_string();

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "session create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let book_id = obj
            .get("book_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("session missing book_id".to_string()))?;
        let started_at = obj
            .get("started_at")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("session missing started_at".to_string()))?;
        let finished_at = obj.get("finished_at").and_then(|v| v.as_str());
        let start_page = obj.get("start_page").and_then(|v| v.as_i64());
        let end_page = obj.get("end_page").and_then(|v| v.as_i64());
        let rating = obj.get("rating").and_then(|v| v.as_i64());
        let notes = obj.get("notes").and_then(|v| v.as_str());
        let now = op.created_at.to_rfc3339();

        let count = self.db.conn.execute(
            "INSERT OR IGNORE INTO reading_sessions
             (id, book_id, started_at, finished_at, start_page, end_page, rating, notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                book_id,
                started_at,
                finished_at,
                start_page,
                end_page,
                rating,
                notes,
                now,
            ],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "session already exists",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Progress — Monotonic (never decrease)
    // -----------------------------------------------------------------------

    fn merge_progress(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_progress_create(op),
            OpType::Update | OpType::Delete => Ok(MergeOutcome::Skipped {
                reason: "progress entries are immutable",
            }),
        }
    }

    fn merge_progress_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let progress_id = op.entity_id.to_string();

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "progress create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let book_id = obj
            .get("book_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("progress missing book_id".to_string()))?;
        let progress_type = obj
            .get("progress_type")
            .and_then(|v| v.as_str())
            .unwrap_or("page");
        let value = obj
            .get("value")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| DbError::InvalidOperation("progress missing value".to_string()))?;
        let session_id = obj.get("session_id").and_then(|v| v.as_str());
        let note = obj.get("note").and_then(|v| v.as_str());
        let logged_at = obj
            .get("logged_at")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("progress missing logged_at".to_string()))?;
        let now = op.created_at.to_rfc3339();

        // Check monotonic constraint: only insert if value > current max for
        // this book and progress_type
        let current_max: Option<i64> = self
            .db
            .conn
            .query_row(
                "SELECT MAX(value) FROM reading_progress
                 WHERE book_id = ?1 AND progress_type = ?2",
                params![book_id, progress_type],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        if let Some(max) = current_max
            && value <= max
        {
            return Ok(MergeOutcome::Skipped {
                reason: "progress value not higher than current",
            });
        }

        // Also skip if this exact progress_id already exists
        let count = self.db.conn.execute(
            "INSERT OR IGNORE INTO reading_progress
             (id, book_id, session_id, progress_type, value, note, logged_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                progress_id,
                book_id,
                session_id,
                progress_type,
                value,
                note,
                logged_at,
                now,
            ],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "progress entry already exists",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Tag — Apply add/remove ops
    // -----------------------------------------------------------------------

    fn merge_tag(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_tag_create(op),
            OpType::Delete => self.merge_tag_delete(op),
            OpType::Update => Ok(MergeOutcome::Skipped {
                reason: "tag update not supported",
            }),
        }
    }

    fn merge_tag_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let book_id = op.entity_id.to_string();

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "tag create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let tag_name = obj
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("tag op missing tag_name".to_string()))?;
        let tag_type = obj
            .get("tag_type")
            .and_then(|v| v.as_str())
            .unwrap_or("general");
        let now = chrono::Utc::now().to_rfc3339();

        // Ensure tag exists (tags table has tag_type column since V9)
        let tag_id = Uuid::now_v7().to_string();
        self.db.conn.execute(
            "INSERT OR IGNORE INTO tags (id, name, tag_type, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![tag_id, tag_name, tag_type, now],
        )?;

        // Get the actual tag_id (may differ if tag already existed)
        let actual_tag_id: String = self.db.conn.query_row(
            "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE AND tag_type = ?2",
            params![tag_name, tag_type],
            |row| row.get(0),
        )?;

        // Add tag to book
        let count = self.db.conn.execute(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
            params![book_id, actual_tag_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "tag already on book",
            })
        }
    }

    fn merge_tag_delete(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let book_id = op.entity_id.to_string();

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "tag delete op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let tag_name = obj
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("tag op missing tag_name".to_string()))?;
        let tag_type = obj
            .get("tag_type")
            .and_then(|v| v.as_str())
            .unwrap_or("general");

        // Find tag_id by name and type
        let tag_id: Option<String> = self
            .db
            .conn
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE AND tag_type = ?2",
                params![tag_name, tag_type],
                |row| row.get(0),
            )
            .optional()?;

        let Some(tag_id) = tag_id else {
            return Ok(MergeOutcome::Skipped {
                reason: "tag not found",
            });
        };

        let count = self.db.conn.execute(
            "DELETE FROM book_tags WHERE book_id = ?1 AND tag_id = ?2",
            params![book_id, tag_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "tag was not on book",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Note — LWW with conflict detection
    // -----------------------------------------------------------------------

    fn merge_note(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_note_create(op),
            OpType::Update => self.merge_note_update(op),
            OpType::Delete => self.merge_note_delete(op),
        }
    }

    fn merge_note_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let note_id = op.entity_id.to_string();

        let exists: bool = self.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1)",
            params![note_id],
            |row| row.get(0),
        )?;
        if exists {
            return self.merge_note_update(op);
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "note create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let book_id = obj
            .get("book_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("note missing book_id".to_string()))?;
        let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let now = op.created_at.to_rfc3339();

        self.db.conn.execute(
            "INSERT INTO notes (id, book_id, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![note_id, book_id, content, now, now],
        )?;

        Ok(MergeOutcome::Applied)
    }

    fn merge_note_update(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let note_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();
        let device_id = op.device_id.to_string();

        // Check note exists and is not deleted
        let row: Option<(String, bool)> = self
            .db
            .conn
            .query_row(
                "SELECT content, deleted_at IS NOT NULL FROM notes WHERE id = ?1",
                params![note_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((local_content, deleted)) = row else {
            return Ok(MergeOutcome::Skipped {
                reason: "note not found",
            });
        };
        if deleted {
            return Ok(MergeOutcome::Skipped {
                reason: "note is deleted",
            });
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Skipped {
                    reason: "update op has no fields",
                });
            }
        };
        let obj = fields.as_object().unwrap();
        let new_content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let (local_hlc, local_device) = self.get_entity_hlc("note", &note_id, "content")?;

        // Determine if this is a genuine conflict (different device edited)
        let is_conflict = local_hlc.is_some()
            && local_device.as_deref() != Some(&device_id)
            && local_content != new_content;

        if let Some(ref lh) = local_hlc
            && hlc_str <= *lh
        {
            // Incoming is older — skip but store conflict if genuine
            if is_conflict {
                let conflict = self.store_conflict(
                    &op.entity_type,
                    &op.entity_id,
                    "content",
                    Some(&local_content),
                    Some(new_content),
                    lh,
                    &hlc_str,
                )?;
                return Ok(MergeOutcome::SkippedWithConflicts(vec![conflict]));
            }
            return Ok(MergeOutcome::Skipped {
                reason: "note content is up to date",
            });
        }

        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "UPDATE notes SET content = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_content, now, note_id],
        )?;
        self.upsert_entity_hlc("note", &note_id, "content", &hlc_str, &device_id)?;

        if is_conflict {
            let conflict = self.store_conflict(
                &op.entity_type,
                &op.entity_id,
                "content",
                Some(&local_content),
                Some(new_content),
                local_hlc.as_deref().unwrap_or(""),
                &hlc_str,
            )?;
            return Ok(MergeOutcome::AppliedWithConflicts(vec![conflict]));
        }

        Ok(MergeOutcome::Applied)
    }

    fn merge_note_delete(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let note_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();
        let device_id = op.device_id.to_string();

        let count = self.db.conn.execute(
            "UPDATE notes SET deleted_at = ?1, deleted_by_device = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            params![hlc_str, device_id, note_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "note not found or already deleted",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Review — LWW per field with conflict detection
    // -----------------------------------------------------------------------

    fn merge_review(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_review_create(op),
            OpType::Update => self.merge_review_update(op),
            OpType::Delete => self.merge_review_delete(op),
        }
    }

    fn merge_review_create(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let review_id = op.entity_id.to_string();

        let exists: bool = self.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM reviews WHERE id = ?1)",
            params![review_id],
            |row| row.get(0),
        )?;
        if exists {
            return self.merge_review_update(op);
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "review create op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let book_id = obj
            .get("book_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("review missing book_id".to_string()))?;
        let content = obj.get("content").and_then(|v| v.as_str());
        let rating = obj.get("rating").and_then(|v| v.as_i64());
        let now = op.created_at.to_rfc3339();

        self.db.conn.execute(
            "INSERT INTO reviews (id, book_id, content, rating, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![review_id, book_id, content, rating, now, now],
        )?;

        Ok(MergeOutcome::Applied)
    }

    fn merge_review_update(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let review_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();
        let device_id = op.device_id.to_string();

        // Check review exists and is not deleted
        let row: Option<(Option<String>, Option<i64>, bool)> = self
            .db
            .conn
            .query_row(
                "SELECT content, rating, deleted_at IS NOT NULL FROM reviews WHERE id = ?1",
                params![review_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let Some((local_content, local_rating, deleted)) = row else {
            return Ok(MergeOutcome::Skipped {
                reason: "review not found",
            });
        };
        if deleted {
            return Ok(MergeOutcome::Skipped {
                reason: "review is deleted",
            });
        }

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Skipped {
                    reason: "update op has no fields",
                });
            }
        };
        let obj = fields.as_object().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let mut applied_any = false;
        let mut conflicts = Vec::new();

        // Field: content
        if let Some(new_content_val) = obj.get("content") {
            let new_content = new_content_val.as_str();
            let (lh, ld) = self.get_entity_hlc("review", &review_id, "content")?;

            let is_conflict = lh.is_some()
                && ld.as_deref() != Some(&device_id)
                && local_content.as_deref() != new_content;

            let should_apply = match &lh {
                Some(lh) => hlc_str > *lh,
                None => true,
            };

            if should_apply {
                self.db.conn.execute(
                    "UPDATE reviews SET content = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_content, now, review_id],
                )?;
                self.upsert_entity_hlc("review", &review_id, "content", &hlc_str, &device_id)?;
                applied_any = true;

                if is_conflict {
                    conflicts.push(self.store_conflict(
                        &op.entity_type,
                        &op.entity_id,
                        "content",
                        local_content.as_deref(),
                        new_content,
                        lh.as_deref().unwrap_or(""),
                        &hlc_str,
                    )?);
                }
            } else if is_conflict {
                conflicts.push(self.store_conflict(
                    &op.entity_type,
                    &op.entity_id,
                    "content",
                    local_content.as_deref(),
                    new_content,
                    lh.as_deref().unwrap_or(""),
                    &hlc_str,
                )?);
            }
        }

        // Field: rating
        if let Some(new_rating_val) = obj.get("rating") {
            let new_rating = new_rating_val.as_i64();
            let (lh, ld) = self.get_entity_hlc("review", &review_id, "rating")?;

            let local_rating_str = local_rating.map(|r| r.to_string());
            let new_rating_str = new_rating.map(|r| r.to_string());
            let is_conflict = lh.is_some()
                && ld.as_deref() != Some(&device_id)
                && local_rating_str != new_rating_str;

            let should_apply = match &lh {
                Some(lh) => hlc_str > *lh,
                None => true,
            };

            if should_apply {
                self.db.conn.execute(
                    "UPDATE reviews SET rating = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_rating, now, review_id],
                )?;
                self.upsert_entity_hlc("review", &review_id, "rating", &hlc_str, &device_id)?;
                applied_any = true;

                if is_conflict {
                    conflicts.push(self.store_conflict(
                        &op.entity_type,
                        &op.entity_id,
                        "rating",
                        local_rating_str.as_deref(),
                        new_rating_str.as_deref(),
                        lh.as_deref().unwrap_or(""),
                        &hlc_str,
                    )?);
                }
            } else if is_conflict {
                conflicts.push(self.store_conflict(
                    &op.entity_type,
                    &op.entity_id,
                    "rating",
                    local_rating_str.as_deref(),
                    new_rating_str.as_deref(),
                    lh.as_deref().unwrap_or(""),
                    &hlc_str,
                )?);
            }
        }

        if !conflicts.is_empty() {
            if applied_any {
                return Ok(MergeOutcome::AppliedWithConflicts(conflicts));
            } else {
                return Ok(MergeOutcome::SkippedWithConflicts(conflicts));
            }
        }
        if applied_any {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "all review fields are up to date",
            })
        }
    }

    fn merge_review_delete(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let review_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();
        let device_id = op.device_id.to_string();

        let count = self.db.conn.execute(
            "UPDATE reviews SET deleted_at = ?1, deleted_by_device = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            params![hlc_str, device_id, review_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "review not found or already deleted",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Setting — LWW per key
    // -----------------------------------------------------------------------

    fn merge_setting(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        match op.op_type {
            OpType::Create => self.merge_setting_upsert(op),
            OpType::Update => self.merge_setting_upsert(op),
            OpType::Delete => self.merge_setting_delete(op),
        }
    }

    fn merge_setting_upsert(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let setting_id = op.entity_id.to_string();
        let hlc_str = op.hlc.to_canonical();

        let fields = match &op.fields {
            Some(v) if v.is_object() => v,
            _ => {
                return Ok(MergeOutcome::Rejected {
                    reason: "setting op missing fields".to_string(),
                });
            }
        };
        let obj = fields.as_object().unwrap();

        let key = obj
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("setting missing key".to_string()))?;
        let value = obj
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DbError::InvalidOperation("setting missing value".to_string()))?;

        // Check if setting exists and compare HLC
        let existing: Option<(String, Option<String>)> = self
            .db
            .conn
            .query_row(
                "SELECT value, sync_hlc FROM user_settings WHERE id = ?1",
                params![setting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((_, Some(ref lh))) = existing
            && hlc_str <= *lh
        {
            return Ok(MergeOutcome::Skipped {
                reason: "setting is up to date",
            });
        }

        let now = chrono::Utc::now().to_rfc3339();
        self.db.conn.execute(
            "INSERT INTO user_settings (id, key, value, sync_hlc, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET value = ?3, sync_hlc = ?4, updated_at = ?5
             WHERE sync_hlc IS NULL OR sync_hlc < ?4",
            params![setting_id, key, value, hlc_str, now],
        )?;

        Ok(MergeOutcome::Applied)
    }

    fn merge_setting_delete(&self, op: &SyncOp) -> Result<MergeOutcome, DbError> {
        let setting_id = op.entity_id.to_string();

        let count = self.db.conn.execute(
            "DELETE FROM user_settings WHERE id = ?1",
            params![setting_id],
        )?;

        if count > 0 {
            Ok(MergeOutcome::Applied)
        } else {
            Ok(MergeOutcome::Skipped {
                reason: "setting not found",
            })
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Upsert a provenance record with the sync HLC.
    fn upsert_provenance(&self, book_id: &str, field_name: &str, hlc: &str) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT INTO metadata_provenance (book_id, field_name, source, source_date, is_user_override, sync_hlc)
             VALUES (?1, ?2, 'sync', ?3, 0, ?3)
             ON CONFLICT (book_id, field_name) DO UPDATE SET sync_hlc = ?3
             WHERE sync_hlc IS NULL OR sync_hlc < ?3",
            params![book_id, field_name, hlc],
        )?;
        Ok(())
    }

    /// Upsert an entity-level HLC record for non-book entities.
    fn upsert_entity_hlc(
        &self,
        entity_type: &str,
        entity_id: &str,
        field_name: &str,
        hlc: &str,
        device_id: &str,
    ) -> Result<(), DbError> {
        self.db.conn.execute(
            "INSERT INTO sync_entity_hlc (entity_type, entity_id, field_name, sync_hlc, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (entity_type, entity_id, field_name) DO UPDATE
             SET sync_hlc = ?4, device_id = ?5
             WHERE sync_hlc < ?4",
            params![entity_type, entity_id, field_name, hlc, device_id],
        )?;
        Ok(())
    }

    /// Get the current HLC and device for an entity field.
    fn get_entity_hlc(
        &self,
        entity_type: &str,
        entity_id: &str,
        field_name: &str,
    ) -> Result<(Option<String>, Option<String>), DbError> {
        let result: Option<(String, Option<String>)> = self
            .db
            .conn
            .query_row(
                "SELECT sync_hlc, device_id FROM sync_entity_hlc
                 WHERE entity_type = ?1 AND entity_id = ?2 AND field_name = ?3",
                params![entity_type, entity_id, field_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        match result {
            Some((hlc, device)) => Ok((Some(hlc), device)),
            None => Ok((None, None)),
        }
    }

    /// Store a conflict in the sync_conflicts table for user review.
    #[allow(clippy::too_many_arguments)]
    fn store_conflict(
        &self,
        entity_type: &EntityType,
        entity_id: &Uuid,
        field_name: &str,
        local_value: Option<&str>,
        remote_value: Option<&str>,
        local_hlc: &str,
        remote_hlc: &str,
    ) -> Result<MergeConflict, DbError> {
        let conflict_id = Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.db.conn.execute(
            "INSERT OR IGNORE INTO sync_conflicts
             (id, entity_type, entity_id, field_name, local_value, remote_value,
              local_hlc, remote_hlc, resolved, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
            params![
                conflict_id,
                entity_type.as_str(),
                entity_id.to_string(),
                field_name,
                local_value,
                remote_value,
                local_hlc,
                remote_hlc,
                now,
            ],
        )?;

        Ok(MergeConflict {
            entity_type: *entity_type,
            entity_id: *entity_id,
            field_name: field_name.to_string(),
            local_value: local_value.map(|s| s.to_string()),
            remote_value: remote_value.map(|s| s.to_string()),
            local_hlc: local_hlc.to_string(),
            remote_hlc: remote_hlc.to_string(),
        })
    }
}

/// Convert a serde_json::Value to a string suitable for SQL binding.
fn value_to_sql(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b { "1" } else { "0" }.to_string()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toku_core::sync::{HlcTimestamp, HybridClock, SyncOp};

    fn setup_db() -> Database {
        Database::open_in_memory().expect("in-memory DB")
    }

    fn make_clock(device_id: &uuid::Uuid) -> HybridClock {
        HybridClock::new(device_id)
    }

    fn make_book_create(
        device_id: uuid::Uuid,
        hlc: HlcTimestamp,
        book_id: uuid::Uuid,
        title: &str,
    ) -> SyncOp {
        SyncOp::new(
            device_id,
            hlc,
            EntityType::Book,
            book_id,
            OpType::Create,
            Some(serde_json::json!({
                "title": title,
                "status": "want-to-read",
                "format": "physical",
            })),
        )
    }

    fn make_book_update(
        device_id: uuid::Uuid,
        hlc: HlcTimestamp,
        book_id: uuid::Uuid,
        fields: serde_json::Value,
    ) -> SyncOp {
        SyncOp::new(
            device_id,
            hlc,
            EntityType::Book,
            book_id,
            OpType::Update,
            Some(fields),
        )
    }

    // -------------------------------------------------------------------
    // Book — field-level merge
    // -------------------------------------------------------------------

    #[test]
    fn book_create_inserts_new_book() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let op = make_book_create(dev_a, clock.now(), book_id, "Dune");
        let result = engine.apply_op(&op).unwrap();
        assert!(result.was_applied());

        let title: String = db
            .conn
            .query_row(
                "SELECT title FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Dune");
    }

    #[test]
    fn book_different_fields_no_conflict() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();

        // Create book
        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A updates title
        let op_a = make_book_update(
            dev_a,
            clock_a.now(),
            book_id,
            serde_json::json!({"title": "Dune (Revised)"}),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B updates rating
        let op_b = make_book_update(
            dev_b,
            clock_b.now(),
            book_id,
            serde_json::json!({"rating": 9}),
        );
        let result = engine.apply_op(&op_b).unwrap();
        assert!(result.was_applied());

        // Both fields should be applied
        let (title, rating): (String, Option<i64>) = db
            .conn
            .query_row(
                "SELECT title, rating FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "Dune (Revised)");
        assert_eq!(rating, Some(9));
    }

    #[test]
    fn book_lww_later_hlc_wins() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();

        // Create book with deterministic HLC (earliest)
        let hlc_create = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_create, book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A updates title at T1 (deterministic)
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = make_book_update(
            dev_a,
            hlc_a.clone(),
            book_id,
            serde_json::json!({"title": "Title from A"}),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B updates title at T2 (later — deterministic)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        assert!(
            hlc_b.to_canonical() > hlc_a.to_canonical(),
            "B's HLC should be later"
        );
        let op_b = make_book_update(
            dev_b,
            hlc_b,
            book_id,
            serde_json::json!({"title": "Title from B"}),
        );
        let result = engine.apply_op(&op_b).unwrap();
        assert!(result.was_applied());

        let title: String = db
            .conn
            .query_row(
                "SELECT title FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Title from B");
    }

    #[test]
    fn book_lww_older_hlc_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();

        // Create book with deterministic HLC (earliest)
        let hlc_create = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_create, book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device B updates title at T1 (earlier — deterministic)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = make_book_update(
            dev_b,
            hlc_b,
            book_id,
            serde_json::json!({"title": "Title from B"}),
        );
        engine.apply_op(&op_b).unwrap();

        // Device A updates title at T2 (later — deterministic)
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = make_book_update(
            dev_a,
            hlc_a,
            book_id,
            serde_json::json!({"title": "Title from A"}),
        );
        engine.apply_op(&op_a).unwrap();

        // Now try to apply an old B op — should be skipped
        let old_hlc = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_old = make_book_update(
            dev_b,
            old_hlc,
            book_id,
            serde_json::json!({"title": "Very old title"}),
        );
        let result = engine.apply_op(&op_old).unwrap();
        assert!(
            matches!(result, MergeOutcome::Skipped { .. }),
            "old HLC should be skipped"
        );

        let title: String = db
            .conn
            .query_row(
                "SELECT title FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Title from A");
    }

    #[test]
    fn book_delete_sets_deleted_at() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        // Create book
        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Delete it
        let del_hlc = clock_a.now();
        let del = SyncOp::new(
            dev_a,
            del_hlc.clone(),
            EntityType::Book,
            book_id,
            OpType::Delete,
            None,
        );
        let result = engine.apply_op(&del).unwrap();
        assert!(result.was_applied());

        let deleted_at: Option<String> = db
            .conn
            .query_row(
                "SELECT deleted_at FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(deleted_at.is_some());
        assert_eq!(deleted_at.unwrap(), del_hlc.to_canonical());
    }

    #[test]
    fn book_update_after_delete_is_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let del = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Book,
            book_id,
            OpType::Delete,
            None,
        );
        engine.apply_op(&del).unwrap();

        let update = make_book_update(
            dev_a,
            clock_a.now(),
            book_id,
            serde_json::json!({"title": "New"}),
        );
        let result = engine.apply_op(&update).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { reason } if reason == "book is deleted"));
    }

    // -------------------------------------------------------------------
    // Status transitions
    // -------------------------------------------------------------------

    #[test]
    fn status_valid_transition_applies() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // want-to-read → reading (valid)
        let op = make_book_update(
            dev_a,
            clock_a.now(),
            book_id,
            serde_json::json!({"status": "reading"}),
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(result.was_applied());

        let status: String = db
            .conn
            .query_row(
                "SELECT status FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "reading");
    }

    #[test]
    fn status_invalid_transition_rejected() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // want-to-read → read (invalid — must go through reading first)
        let op = make_book_update(
            dev_a,
            clock_a.now(),
            book_id,
            serde_json::json!({"status": "read"}),
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(matches!(result, MergeOutcome::Rejected { .. }));

        // Status should still be want-to-read
        let status: String = db
            .conn
            .query_row(
                "SELECT status FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "want-to-read");
    }

    // -------------------------------------------------------------------
    // Session — append-only
    // -------------------------------------------------------------------

    #[test]
    fn session_create_inserts_new() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();

        // Create book first
        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let session_op = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "started_at": "2026-06-01T10:00:00Z",
                "notes": "Great start",
            })),
        );
        let result = engine.apply_op(&session_op).unwrap();
        assert!(result.was_applied());

        let notes: Option<String> = db
            .conn
            .query_row(
                "SELECT notes FROM reading_sessions WHERE id = ?1",
                params![session_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(notes.as_deref(), Some("Great start"));
    }

    #[test]
    fn session_duplicate_is_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let session_op = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "started_at": "2026-06-01T10:00:00Z",
            })),
        );
        engine.apply_op(&session_op).unwrap();

        // Same session again
        let dup = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "started_at": "2026-06-01T10:00:00Z",
                "notes": "Different notes",
            })),
        );
        let result = engine.apply_op(&dup).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { .. }));
    }

    #[test]
    fn session_update_and_delete_are_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let session_id = Uuid::now_v7();

        let update = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_id,
            OpType::Update,
            None,
        );
        assert!(matches!(
            engine.apply_op(&update).unwrap(),
            MergeOutcome::Skipped { .. }
        ));

        let delete = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_id,
            OpType::Delete,
            None,
        );
        assert!(matches!(
            engine.apply_op(&delete).unwrap(),
            MergeOutcome::Skipped { .. }
        ));
    }

    // -------------------------------------------------------------------
    // Progress — monotonic
    // -------------------------------------------------------------------

    #[test]
    fn progress_higher_value_applied() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();

        // Create book
        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A logs page 50
        let prog_a = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 50,
                "logged_at": "2026-06-01T10:00:00Z",
            })),
        );
        let result = engine.apply_op(&prog_a).unwrap();
        assert!(result.was_applied());

        // Device B logs page 100 (higher — should apply)
        let prog_b = SyncOp::new(
            dev_b,
            clock_b.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 100,
                "logged_at": "2026-06-01T12:00:00Z",
            })),
        );
        let result = engine.apply_op(&prog_b).unwrap();
        assert!(result.was_applied());
    }

    #[test]
    fn progress_lower_value_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Log page 100 first
        let prog_high = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 100,
                "logged_at": "2026-06-01T12:00:00Z",
            })),
        );
        engine.apply_op(&prog_high).unwrap();

        // Try to log page 50 (lower — should be skipped)
        let prog_low = SyncOp::new(
            dev_b,
            clock_b.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 50,
                "logged_at": "2026-06-01T10:00:00Z",
            })),
        );
        let result = engine.apply_op(&prog_low).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { .. }));

        // Only one progress entry should exist
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM reading_progress WHERE book_id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // -------------------------------------------------------------------
    // Tag — add/remove
    // -------------------------------------------------------------------

    #[test]
    fn tag_create_adds_to_book() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let tag_op = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Tag,
            book_id,
            OpType::Create,
            Some(serde_json::json!({
                "tag_name": "sci-fi",
                "tag_type": "general",
            })),
        );
        let result = engine.apply_op(&tag_op).unwrap();
        assert!(result.was_applied());

        let tag_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM book_tags bt
                 JOIN tags t ON bt.tag_id = t.id
                 WHERE bt.book_id = ?1 AND t.name = 'sci-fi'",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 1);
    }

    #[test]
    fn tag_delete_removes_from_book() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Add tag
        let add = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Tag,
            book_id,
            OpType::Create,
            Some(serde_json::json!({"tag_name": "sci-fi"})),
        );
        engine.apply_op(&add).unwrap();

        // Remove tag
        let remove = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Tag,
            book_id,
            OpType::Delete,
            Some(serde_json::json!({"tag_name": "sci-fi"})),
        );
        let result = engine.apply_op(&remove).unwrap();
        assert!(result.was_applied());

        let tag_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM book_tags WHERE book_id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 0);
    }

    // -------------------------------------------------------------------
    // Two-device scenarios
    // -------------------------------------------------------------------

    #[test]
    fn two_devices_edit_different_book_fields() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();

        // Device A creates book
        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A edits description
        let op_a = make_book_update(
            dev_a,
            clock_a.now(),
            book_id,
            serde_json::json!({"description": "A desert planet saga"}),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B edits page_count
        let op_b = make_book_update(
            dev_b,
            clock_b.now(),
            book_id,
            serde_json::json!({"page_count": 896}),
        );
        engine.apply_op(&op_b).unwrap();

        let (desc, pages): (Option<String>, Option<i64>) = db
            .conn
            .query_row(
                "SELECT description, page_count FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(desc.as_deref(), Some("A desert planet saga"));
        assert_eq!(pages, Some(896));
    }

    #[test]
    fn two_devices_same_field_last_write_wins() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Simulate: device A writes rating=7 at T1 (earlier — deterministic)
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = make_book_update(dev_a, hlc_a, book_id, serde_json::json!({"rating": 7}));
        engine.apply_op(&op_a).unwrap();

        // Device B writes rating=9 at T2 (later — deterministic)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = make_book_update(dev_b, hlc_b, book_id, serde_json::json!({"rating": 9}));
        engine.apply_op(&op_b).unwrap();

        let rating: i64 = db
            .conn
            .query_row(
                "SELECT rating FROM books WHERE id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rating, 9);
    }

    #[test]
    fn two_devices_session_both_append() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();
        let session_a = Uuid::now_v7();
        let session_b = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A creates a session
        let sa = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Session,
            session_a,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "started_at": "2026-06-01T10:00:00Z",
            })),
        );
        engine.apply_op(&sa).unwrap();

        // Device B creates a different session
        let sb = SyncOp::new(
            dev_b,
            clock_b.now(),
            EntityType::Session,
            session_b,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "started_at": "2026-06-02T10:00:00Z",
            })),
        );
        engine.apply_op(&sb).unwrap();

        // Both sessions exist
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM reading_sessions WHERE book_id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn two_devices_progress_monotonic_across_devices() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A: page 100
        let pa = SyncOp::new(
            dev_a,
            clock_a.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 100,
                "logged_at": "2026-06-01T10:00:00Z",
            })),
        );
        engine.apply_op(&pa).unwrap();

        // Device B: page 50 (lower — skipped)
        let pb_low = SyncOp::new(
            dev_b,
            clock_b.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 50,
                "logged_at": "2026-06-01T08:00:00Z",
            })),
        );
        let result = engine.apply_op(&pb_low).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { .. }));

        // Device B: page 200 (higher — applied)
        let pb_high = SyncOp::new(
            dev_b,
            clock_b.now(),
            EntityType::Progress,
            Uuid::now_v7(),
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "progress_type": "page",
                "value": 200,
                "logged_at": "2026-06-02T10:00:00Z",
            })),
        );
        let result = engine.apply_op(&pb_high).unwrap();
        assert!(result.was_applied());

        // 2 progress entries (100, 200); the 50 was skipped
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM reading_progress WHERE book_id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    // -------------------------------------------------------------------
    // Note — LWW with conflict detection
    // -------------------------------------------------------------------

    fn make_note_create(
        device_id: uuid::Uuid,
        hlc: HlcTimestamp,
        note_id: uuid::Uuid,
        book_id: uuid::Uuid,
        content: &str,
    ) -> SyncOp {
        SyncOp::new(
            device_id,
            hlc,
            EntityType::Note,
            note_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "content": content,
            })),
        )
    }

    fn make_note_update(
        device_id: uuid::Uuid,
        hlc: HlcTimestamp,
        note_id: uuid::Uuid,
        content: &str,
    ) -> SyncOp {
        SyncOp::new(
            device_id,
            hlc,
            EntityType::Note,
            note_id,
            OpType::Update,
            Some(serde_json::json!({
                "content": content,
            })),
        )
    }

    #[test]
    fn note_create_inserts_new() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let op = make_note_create(dev_a, clock.now(), note_id, book_id, "Great book");
        let result = engine.apply_op(&op).unwrap();
        assert!(result.was_applied());
        assert!(!result.has_conflicts());

        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Great book");
    }

    #[test]
    fn note_create_missing_fields_rejected() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);

        let op = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Note,
            Uuid::now_v7(),
            OpType::Create,
            None,
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(matches!(result, MergeOutcome::Rejected { .. }));
    }

    #[test]
    fn note_update_from_same_device_no_conflict() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let op = make_note_create(dev_a, clock.now(), note_id, book_id, "Draft 1");
        engine.apply_op(&op).unwrap();

        // Same device updates note — no conflict
        let update = make_note_update(dev_a, clock.now(), note_id, "Draft 2");
        let result = engine.apply_op(&update).unwrap();
        assert!(result.was_applied());
        assert!(!result.has_conflicts());

        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Draft 2");
    }

    #[test]
    fn note_two_devices_conflict_detected() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        // Device A creates book and note
        let hlc_create = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_create.clone(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let hlc_note = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let note_create = make_note_create(dev_a, hlc_note, note_id, book_id, "Original");
        engine.apply_op(&note_create).unwrap();

        // Device A updates note at T1
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = make_note_update(dev_a, hlc_a, note_id, "Edit from A");
        engine.apply_op(&op_a).unwrap();

        // Device B updates note at T2 (later) — conflict: both devices edited
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = make_note_update(dev_b, hlc_b, note_id, "Edit from B");
        let result = engine.apply_op(&op_b).unwrap();

        // B's edit wins (newer HLC), but conflict is stored
        assert!(result.was_applied());
        assert!(result.has_conflicts());
        let conflicts = result.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field_name, "content");
        assert_eq!(conflicts[0].local_value.as_deref(), Some("Edit from A"));
        assert_eq!(conflicts[0].remote_value.as_deref(), Some("Edit from B"));

        // Note content should be B's edit
        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Edit from B");

        // Conflict should be in sync_conflicts table
        let conflict_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_conflicts
                 WHERE entity_type = 'note' AND entity_id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(conflict_count, 1);
    }

    #[test]
    fn note_two_devices_older_edit_skipped_with_conflict() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        // Setup: create book and note
        let hlc_0 = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_0.clone(), book_id, "Dune");
        engine.apply_op(&create).unwrap();
        let note_op = make_note_create(dev_a, hlc_0, note_id, book_id, "Original");
        engine.apply_op(&note_op).unwrap();

        // Device A edits at T2 (later)
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = make_note_update(dev_a, hlc_a, note_id, "Latest from A");
        engine.apply_op(&op_a).unwrap();

        // Device B edits at T1 (earlier) — arrives after A's edit
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = make_note_update(dev_b, hlc_b, note_id, "Older from B");
        let result = engine.apply_op(&op_b).unwrap();

        // B's edit is skipped (older HLC), but conflict is stored
        assert!(!result.was_applied());
        assert!(result.has_conflicts());
        assert!(matches!(result, MergeOutcome::SkippedWithConflicts(_)));

        // Note content should still be A's edit
        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM notes WHERE id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Latest from A");
    }

    #[test]
    fn note_delete_sets_tombstone() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();
        let note = make_note_create(dev_a, clock.now(), note_id, book_id, "A note");
        engine.apply_op(&note).unwrap();

        let del = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Note,
            note_id,
            OpType::Delete,
            None,
        );
        let result = engine.apply_op(&del).unwrap();
        assert!(result.was_applied());

        let deleted: bool = db
            .conn
            .query_row(
                "SELECT deleted_at IS NOT NULL FROM notes WHERE id = ?1",
                params![note_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(deleted);
    }

    #[test]
    fn note_update_after_delete_is_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let note_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();
        let note = make_note_create(dev_a, clock.now(), note_id, book_id, "A note");
        engine.apply_op(&note).unwrap();

        let del = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Note,
            note_id,
            OpType::Delete,
            None,
        );
        engine.apply_op(&del).unwrap();

        let update = make_note_update(dev_a, clock.now(), note_id, "Updated after delete");
        let result = engine.apply_op(&update).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { reason } if reason == "note is deleted"));
    }

    // -------------------------------------------------------------------
    // Review — LWW per field with conflict detection
    // -------------------------------------------------------------------

    #[test]
    fn review_create_inserts_new() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let review_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let op = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Review,
            review_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "content": "Amazing book",
                "rating": 9,
            })),
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(result.was_applied());

        let (content, rating): (Option<String>, Option<i64>) = db
            .conn
            .query_row(
                "SELECT content, rating FROM reviews WHERE id = ?1",
                params![review_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content.as_deref(), Some("Amazing book"));
        assert_eq!(rating, Some(9));
    }

    #[test]
    fn review_two_devices_different_fields_no_conflict() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();
        let review_id = Uuid::now_v7();

        // Create book and review
        let hlc_0 = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_0.clone(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let review_create = SyncOp::new(
            dev_a,
            hlc_0,
            EntityType::Review,
            review_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "content": "Good",
                "rating": 7,
            })),
        );
        engine.apply_op(&review_create).unwrap();

        // Device A edits content
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = SyncOp::new(
            dev_a,
            hlc_a,
            EntityType::Review,
            review_id,
            OpType::Update,
            Some(serde_json::json!({"content": "Updated review text"})),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B edits rating (different field — no conflict)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = SyncOp::new(
            dev_b,
            hlc_b,
            EntityType::Review,
            review_id,
            OpType::Update,
            Some(serde_json::json!({"rating": 9})),
        );
        let result = engine.apply_op(&op_b).unwrap();
        assert!(result.was_applied());
        assert!(!result.has_conflicts());

        let (content, rating): (Option<String>, Option<i64>) = db
            .conn
            .query_row(
                "SELECT content, rating FROM reviews WHERE id = ?1",
                params![review_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content.as_deref(), Some("Updated review text"));
        assert_eq!(rating, Some(9));
    }

    #[test]
    fn review_two_devices_same_field_conflict_detected() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let book_id = Uuid::now_v7();
        let review_id = Uuid::now_v7();

        let hlc_0 = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let create = make_book_create(dev_a, hlc_0.clone(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let review_create = SyncOp::new(
            dev_a,
            hlc_0,
            EntityType::Review,
            review_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "content": "Original",
                "rating": 7,
            })),
        );
        engine.apply_op(&review_create).unwrap();

        // Device A edits content
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = SyncOp::new(
            dev_a,
            hlc_a,
            EntityType::Review,
            review_id,
            OpType::Update,
            Some(serde_json::json!({"content": "Review from A"})),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B also edits content (later HLC — wins, but conflict stored)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = SyncOp::new(
            dev_b,
            hlc_b,
            EntityType::Review,
            review_id,
            OpType::Update,
            Some(serde_json::json!({"content": "Review from B"})),
        );
        let result = engine.apply_op(&op_b).unwrap();

        assert!(result.was_applied());
        assert!(result.has_conflicts());
        let conflicts = result.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field_name, "content");

        let content: String = db
            .conn
            .query_row(
                "SELECT content FROM reviews WHERE id = ?1",
                params![review_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(content, "Review from B");
    }

    #[test]
    fn review_delete_sets_tombstone() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let book_id = Uuid::now_v7();
        let review_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        let review = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Review,
            review_id,
            OpType::Create,
            Some(serde_json::json!({
                "book_id": book_id.to_string(),
                "content": "Great",
            })),
        );
        engine.apply_op(&review).unwrap();

        let del = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Review,
            review_id,
            OpType::Delete,
            None,
        );
        let result = engine.apply_op(&del).unwrap();
        assert!(result.was_applied());

        let deleted: bool = db
            .conn
            .query_row(
                "SELECT deleted_at IS NOT NULL FROM reviews WHERE id = ?1",
                params![review_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(deleted);
    }

    // -------------------------------------------------------------------
    // Setting — LWW per key
    // -------------------------------------------------------------------

    #[test]
    fn setting_create_inserts_new() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let setting_id = Uuid::now_v7();

        let op = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Setting,
            setting_id,
            OpType::Create,
            Some(serde_json::json!({
                "key": "theme",
                "value": "dark",
            })),
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(result.was_applied());

        let value: String = db
            .conn
            .query_row(
                "SELECT value FROM user_settings WHERE id = ?1",
                params![setting_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "dark");
    }

    #[test]
    fn setting_lww_later_wins() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let setting_id = Uuid::now_v7();

        // Device A sets theme=dark at T1
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = SyncOp::new(
            dev_a,
            hlc_a,
            EntityType::Setting,
            setting_id,
            OpType::Create,
            Some(serde_json::json!({"key": "theme", "value": "dark"})),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B sets theme=light at T2 (later — wins)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = SyncOp::new(
            dev_b,
            hlc_b,
            EntityType::Setting,
            setting_id,
            OpType::Update,
            Some(serde_json::json!({"key": "theme", "value": "light"})),
        );
        let result = engine.apply_op(&op_b).unwrap();
        assert!(result.was_applied());

        let value: String = db
            .conn
            .query_row(
                "SELECT value FROM user_settings WHERE id = ?1",
                params![setting_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "light");
    }

    #[test]
    fn setting_lww_older_skipped() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let setting_id = Uuid::now_v7();

        // Device A sets theme=dark at T2 (later)
        let hlc_a = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "aaaaaaaaaaaa",
        );
        let op_a = SyncOp::new(
            dev_a,
            hlc_a,
            EntityType::Setting,
            setting_id,
            OpType::Create,
            Some(serde_json::json!({"key": "theme", "value": "dark"})),
        );
        engine.apply_op(&op_a).unwrap();

        // Device B sets theme=light at T1 (earlier — skipped)
        let hlc_b = HlcTimestamp::new(
            chrono::DateTime::parse_from_rfc3339("2025-06-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            0,
            "bbbbbbbbbbbb",
        );
        let op_b = SyncOp::new(
            dev_b,
            hlc_b,
            EntityType::Setting,
            setting_id,
            OpType::Update,
            Some(serde_json::json!({"key": "theme", "value": "light"})),
        );
        let result = engine.apply_op(&op_b).unwrap();
        assert!(matches!(result, MergeOutcome::Skipped { .. }));

        let value: String = db
            .conn
            .query_row(
                "SELECT value FROM user_settings WHERE id = ?1",
                params![setting_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "dark");
    }

    #[test]
    fn setting_delete_removes() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);
        let setting_id = Uuid::now_v7();

        let create = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Setting,
            setting_id,
            OpType::Create,
            Some(serde_json::json!({"key": "theme", "value": "dark"})),
        );
        engine.apply_op(&create).unwrap();

        let del = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Setting,
            setting_id,
            OpType::Delete,
            None,
        );
        let result = engine.apply_op(&del).unwrap();
        assert!(result.was_applied());

        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM user_settings WHERE id = ?1",
                params![setting_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn setting_missing_fields_rejected() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock = make_clock(&dev_a);

        let op = SyncOp::new(
            dev_a,
            clock.now(),
            EntityType::Setting,
            Uuid::now_v7(),
            OpType::Create,
            None,
        );
        let result = engine.apply_op(&op).unwrap();
        assert!(matches!(result, MergeOutcome::Rejected { .. }));
    }

    // -------------------------------------------------------------------
    // Two-device note scenarios
    // -------------------------------------------------------------------

    #[test]
    fn two_devices_edit_different_notes_no_conflict() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let mut clock_b = make_clock(&dev_b);
        let book_id = Uuid::now_v7();
        let note_a = Uuid::now_v7();
        let note_b = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");
        engine.apply_op(&create).unwrap();

        // Device A creates note A
        let op_a = make_note_create(dev_a, clock_a.now(), note_a, book_id, "Note A");
        engine.apply_op(&op_a).unwrap();

        // Device B creates note B (different note — no conflict)
        let op_b = make_note_create(dev_b, clock_b.now(), note_b, book_id, "Note B");
        let result = engine.apply_op(&op_b).unwrap();
        assert!(result.was_applied());
        assert!(!result.has_conflicts());

        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE book_id = ?1",
                params![book_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn apply_op_is_idempotent() {
        let db = setup_db();
        let engine = MergeEngine::new(&db);
        let dev_a = Uuid::now_v7();
        let mut clock_a = make_clock(&dev_a);
        let book_id = Uuid::now_v7();

        let create = make_book_create(dev_a, clock_a.now(), book_id, "Dune");

        // Apply the same create op twice
        let r1 = engine.apply_op(&create).unwrap();
        assert!(r1.was_applied());

        let r2 = engine.apply_op(&create).unwrap();
        // Second time: book exists, so it becomes an update — fields haven't
        // changed so it should be skipped
        assert!(matches!(r2, MergeOutcome::Skipped { .. }));

        // Only one book
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
