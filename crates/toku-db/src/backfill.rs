//! First-opt-in op-backfill (#199, ADR-013 D2).
//!
//! When a user opts into sync, every syncable row created *before* opt-in has no
//! op in the log — a device identity is created *by* opt-in, and #194's
//! op-emission is a no-op until then. Backfill closes that boundary: it
//! synthesizes `Create` ops for the existing rows of the syncable entity types
//! (Book, Session, Progress, Tag — the exact set #194 covers) and stages them so
//! the normal push pipeline uploads them.
//!
//! Design notes:
//!
//! * **Same ops as ongoing sync.** Ops are emitted through the same
//!   [`SyncRepository::emit_local_op`] choke-point normal mutations use, with
//!   field payloads built by the shared `*_op_fields` helpers — there is no
//!   second, divergent "snapshot subset" that could drift from the op stream
//!   (the failure ADR-013 flags with `compact`).
//! * **Dependency order.** Books are emitted before Sessions/Progress/Tags so
//!   the monotonic HLC guarantees a parent Book Create sorts before the child
//!   ops that reference it on the receiving device.
//! * **Idempotent.** An entity that already has a `Create` op is skipped, so
//!   re-running backfill (or backfilling a library that already emitted some ops
//!   through ongoing edits) never duplicates.
//! * **Live rows only.** Soft-deleted books (and their children) are skipped —
//!   there is nothing on the server for a pre-opt-in tombstone to delete.

use std::collections::HashSet;

use toku_core::{EntityType, OpType};

use crate::repo::{book_op_fields, progress_op_fields, session_op_fields, tag_op_fields};
use crate::{BookRepository, Database, DbError, SyncRepository};

/// Per-entity tally of ops synthesized by [`backfill_sync_ops`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillCounts {
    pub books: usize,
    pub sessions: usize,
    pub progress: usize,
    pub tags: usize,
}

impl BackfillCounts {
    /// Total ops synthesized across all entity types.
    pub fn total(&self) -> usize {
        self.books + self.sessions + self.progress + self.tags
    }
}

/// Synthesize `Create` ops for existing syncable rows that don't already have
/// one, staging them for the next push. Returns a per-entity tally.
///
/// A no-op that returns zero counts when no device identity is configured: op
/// emission requires a device (offline-first), and the caller only invokes this
/// at the opt-in boundary once a device exists. Runs in a single transaction so
/// a partial failure rolls back cleanly and can be retried.
pub fn backfill_sync_ops(db: &Database) -> Result<BackfillCounts, DbError> {
    let sync = SyncRepository::new(db);

    // Offline-first: without a device, emit_local_op is a no-op, so there is
    // nothing honest to count. Bail early rather than report phantom ops.
    if sync.get_device()?.is_none() {
        return Ok(BackfillCounts::default());
    }

    if db.conn.is_autocommit() {
        let tx = db.conn.unchecked_transaction()?;
        let counts = backfill_inner(db, &sync)?;
        tx.commit()?;
        Ok(counts)
    } else {
        backfill_inner(db, &sync)
    }
}

fn backfill_inner(db: &Database, sync: &SyncRepository) -> Result<BackfillCounts, DbError> {
    let existing = ExistingCreateOps::load(db)?;
    let repo = BookRepository::new(db);
    let mut counts = BackfillCounts::default();

    // Live books first so their Create ops sort ahead of child ops by HLC.
    let books = repo.list_books()?;
    let live_book_ids: HashSet<String> = books.iter().map(|b| b.id.to_string()).collect();
    for book in &books {
        if existing.books.contains(&book.id.to_string()) {
            continue;
        }
        sync.emit_local_op(
            EntityType::Book,
            book.id,
            OpType::Create,
            Some(book_op_fields(book)),
        )?;
        counts.books += 1;
    }

    // Reading sessions belonging to a live book.
    for session in repo.list_reading_sessions()? {
        if !live_book_ids.contains(&session.book_id.to_string()) {
            continue;
        }
        if existing.sessions.contains(&session.id.to_string()) {
            continue;
        }
        sync.emit_local_op(
            EntityType::Session,
            session.id,
            OpType::Create,
            Some(session_op_fields(&session)),
        )?;
        counts.sessions += 1;
    }

    // Reading progress and tags are read per book (both are keyed on the book).
    for book in &books {
        for progress in repo.get_reading_log(&book.id)? {
            if existing.progress.contains(&progress.id.to_string()) {
                continue;
            }
            sync.emit_local_op(
                EntityType::Progress,
                progress.id,
                OpType::Create,
                Some(progress_op_fields(&progress)),
            )?;
            counts.progress += 1;
        }

        let book_id = book.id.to_string();
        for tag in repo.get_book_tags(&book.id)? {
            let key = (
                book_id.clone(),
                tag.name.clone(),
                tag.tag_type.as_str().to_string(),
            );
            if existing.tags.contains(&key) {
                continue;
            }
            sync.emit_local_op(
                EntityType::Tag,
                book.id,
                OpType::Create,
                Some(tag_op_fields(&tag.name, tag.tag_type)),
            )?;
            counts.tags += 1;
        }
    }

    Ok(counts)
}

/// The set of entities that already carry a `Create` op, used to make backfill
/// idempotent. Book/Session/Progress are keyed by `entity_id`; Tag is keyed by
/// `(book_id, tag_name, tag_type)` since many tag ops share one book id.
struct ExistingCreateOps {
    books: HashSet<String>,
    sessions: HashSet<String>,
    progress: HashSet<String>,
    tags: HashSet<(String, String, String)>,
}

impl ExistingCreateOps {
    fn load(db: &Database) -> Result<Self, DbError> {
        let mut out = Self {
            books: HashSet::new(),
            sessions: HashSet::new(),
            progress: HashSet::new(),
            tags: HashSet::new(),
        };

        let mut stmt = db.conn.prepare(
            "SELECT entity_type, entity_id, fields_json
             FROM sync_ops WHERE op_type = 'create'",
        )?;
        let rows = stmt.query_map([], |row| {
            let entity_type: String = row.get(0)?;
            let entity_id: String = row.get(1)?;
            let fields_json: Option<String> = row.get(2)?;
            Ok((entity_type, entity_id, fields_json))
        })?;

        for row in rows {
            let (entity_type, entity_id, fields_json) = row?;
            match entity_type.as_str() {
                "book" => {
                    out.books.insert(entity_id);
                }
                "session" => {
                    out.sessions.insert(entity_id);
                }
                "progress" => {
                    out.progress.insert(entity_id);
                }
                "tag" => {
                    if let Some((name, ty)) = tag_key_from_fields(fields_json.as_deref()) {
                        out.tags.insert((entity_id, name, ty));
                    }
                }
                _ => {}
            }
        }

        Ok(out)
    }
}

/// Extract `(tag_name, tag_type)` from a Tag op's `fields_json`, if present.
fn tag_key_from_fields(fields_json: Option<&str>) -> Option<(String, String)> {
    let raw = fields_json?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let name = value.get("tag_name")?.as_str()?.to_string();
    let ty = value.get("tag_type")?.as_str()?.to_string();
    Some((name, ty))
}
