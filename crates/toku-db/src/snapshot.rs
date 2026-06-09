//! Snapshot export and import for sync compaction and new-device bootstrap.
//!
//! Export reads the full library state into a [`LibrarySnapshot`].
//! Import applies a snapshot to the local database, inserting all entities.

use base64::Engine as _;
use rusqlite::params;
use toku_core::sync::{LibrarySnapshot, SnapshotLibrary};
use uuid::Uuid;

use crate::{Database, DbError};

/// Snapshot persistence operations.
pub struct SnapshotRepository<'a> {
    db: &'a Database,
}

impl<'a> SnapshotRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Export the full library state as a snapshot.
    pub fn export_snapshot(
        &self,
        device_id: Uuid,
        hlc_at_snapshot: &str,
    ) -> Result<LibrarySnapshot, DbError> {
        let books = self.export_table(
            "SELECT id, title, subtitle, description, page_count, pub_date,
             language, format, duration_minutes, cover_hash, work_id, status,
             rating, created_at, updated_at, deleted_at, deleted_by_device
             FROM books ORDER BY id",
            &[
                "id",
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
                "created_at",
                "updated_at",
                "deleted_at",
                "deleted_by_device",
            ],
        )?;

        let book_authors = self.export_table(
            "SELECT ba.book_id, ba.author_id, ba.role, ba.position,
                    a.name, a.sort_name
             FROM book_authors ba
             JOIN authors a ON a.id = ba.author_id
             ORDER BY ba.book_id, ba.position",
            &[
                "book_id",
                "author_id",
                "role",
                "position",
                "author_name",
                "author_sort_name",
            ],
        )?;

        let sessions = self.export_table(
            "SELECT id, book_id, started_at, finished_at, start_page, end_page,
             rating, notes, created_at
             FROM reading_sessions ORDER BY id",
            &[
                "id",
                "book_id",
                "started_at",
                "finished_at",
                "start_page",
                "end_page",
                "rating",
                "notes",
                "created_at",
            ],
        )?;

        let progress = self.export_table(
            "SELECT id, book_id, session_id, progress_type, value, note,
             logged_at, created_at
             FROM reading_progress ORDER BY id",
            &[
                "id",
                "book_id",
                "session_id",
                "progress_type",
                "value",
                "note",
                "logged_at",
                "created_at",
            ],
        )?;

        let tags = self.export_table(
            "SELECT id, name, tag_type, created_at FROM tags ORDER BY id",
            &["id", "name", "tag_type", "created_at"],
        )?;

        let book_tags = self.export_table(
            "SELECT book_id, tag_id FROM book_tags ORDER BY book_id, tag_id",
            &["book_id", "tag_id"],
        )?;

        let notes = self.export_table(
            "SELECT id, book_id, content, deleted_at, deleted_by_device,
             created_at, updated_at
             FROM notes ORDER BY id",
            &[
                "id",
                "book_id",
                "content",
                "deleted_at",
                "deleted_by_device",
                "created_at",
                "updated_at",
            ],
        )?;

        let reviews = self.export_table(
            "SELECT id, book_id, content, rating, deleted_at, deleted_by_device,
             created_at, updated_at
             FROM reviews ORDER BY id",
            &[
                "id",
                "book_id",
                "content",
                "rating",
                "deleted_at",
                "deleted_by_device",
                "created_at",
                "updated_at",
            ],
        )?;

        let settings = self.export_table(
            "SELECT id, key, value, sync_hlc, updated_at FROM user_settings ORDER BY id",
            &["id", "key", "value", "sync_hlc", "updated_at"],
        )?;

        Ok(LibrarySnapshot {
            version: 1,
            created_at: chrono::Utc::now(),
            created_by_device: device_id,
            hlc_at_snapshot: hlc_at_snapshot.to_string(),
            library: SnapshotLibrary {
                books,
                book_authors,
                sessions,
                progress,
                tags,
                book_tags,
                notes,
                reviews,
                settings,
            },
        })
    }

    /// Apply a snapshot to the local database, inserting all entities.
    ///
    /// This is used for new-device bootstrap. The database should be empty
    /// or the caller should handle conflicts (existing data is not cleared).
    pub fn apply_snapshot(
        &self,
        snapshot: &LibrarySnapshot,
    ) -> Result<SnapshotApplyResult, DbError> {
        let tx = self.db.conn.unchecked_transaction()?;
        let mut result = SnapshotApplyResult::default();

        // Books
        for book in &snapshot.library.books {
            let obj = book.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot book is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO books
                 (id, title, subtitle, description, page_count, pub_date,
                  language, format, duration_minutes, cover_hash, work_id, status,
                  rating, created_at, updated_at, deleted_at, deleted_by_device)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "title"),
                    json_str_opt(obj, "subtitle"),
                    json_str_opt(obj, "description"),
                    json_i64_opt(obj, "page_count"),
                    json_str_opt(obj, "pub_date"),
                    json_str_opt(obj, "language"),
                    json_str(obj, "format"),
                    json_i64_opt(obj, "duration_minutes"),
                    json_str_opt(obj, "cover_hash"),
                    json_str_opt(obj, "work_id"),
                    json_str(obj, "status"),
                    json_i64_opt(obj, "rating"),
                    json_str(obj, "created_at"),
                    json_str(obj, "updated_at"),
                    json_str_opt(obj, "deleted_at"),
                    json_str_opt(obj, "deleted_by_device"),
                ],
            )?;
            result.books += 1;
        }

        // Authors + book_authors
        for ba in &snapshot.library.book_authors {
            let obj = ba.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot book_author is not an object".into())
            })?;
            let author_id = json_str(obj, "author_id");
            let author_name = json_str(obj, "author_name");
            let sort_name = json_str_opt(obj, "author_sort_name");

            tx.execute(
                "INSERT OR IGNORE INTO authors (id, name, sort_name) VALUES (?1, ?2, ?3)",
                params![author_id, author_name, sort_name],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    json_str(obj, "book_id"),
                    author_id,
                    json_str(obj, "role"),
                    json_i64_opt(obj, "position").unwrap_or(0),
                ],
            )?;
        }

        // Sessions
        for session in &snapshot.library.sessions {
            let obj = session.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot session is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO reading_sessions
                 (id, book_id, started_at, finished_at, start_page, end_page,
                  rating, notes, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "book_id"),
                    json_str(obj, "started_at"),
                    json_str_opt(obj, "finished_at"),
                    json_i64_opt(obj, "start_page"),
                    json_i64_opt(obj, "end_page"),
                    json_i64_opt(obj, "rating"),
                    json_str_opt(obj, "notes"),
                    json_str(obj, "created_at"),
                ],
            )?;
            result.sessions += 1;
        }

        // Progress
        for prog in &snapshot.library.progress {
            let obj = prog.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot progress is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO reading_progress
                 (id, book_id, session_id, progress_type, value, note,
                  logged_at, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "book_id"),
                    json_str_opt(obj, "session_id"),
                    json_str(obj, "progress_type"),
                    json_i64_opt(obj, "value").unwrap_or(0),
                    json_str_opt(obj, "note"),
                    json_str(obj, "logged_at"),
                    json_str(obj, "created_at"),
                ],
            )?;
            result.progress += 1;
        }

        // Tags
        for tag in &snapshot.library.tags {
            let obj = tag
                .as_object()
                .ok_or_else(|| DbError::InvalidOperation("snapshot tag is not an object".into()))?;
            tx.execute(
                "INSERT OR IGNORE INTO tags (id, name, tag_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "name"),
                    json_str(obj, "tag_type"),
                    json_str(obj, "created_at"),
                ],
            )?;
            result.tags += 1;
        }

        // Book-tag associations
        for bt in &snapshot.library.book_tags {
            let obj = bt.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot book_tag is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
                params![json_str(obj, "book_id"), json_str(obj, "tag_id")],
            )?;
        }

        // Notes
        for note in &snapshot.library.notes {
            let obj = note.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot note is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO notes
                 (id, book_id, content, deleted_at, deleted_by_device,
                  created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "book_id"),
                    json_str(obj, "content"),
                    json_str_opt(obj, "deleted_at"),
                    json_str_opt(obj, "deleted_by_device"),
                    json_str(obj, "created_at"),
                    json_str(obj, "updated_at"),
                ],
            )?;
            result.notes += 1;
        }

        // Reviews
        for review in &snapshot.library.reviews {
            let obj = review.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot review is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO reviews
                 (id, book_id, content, rating, deleted_at, deleted_by_device,
                  created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "book_id"),
                    json_str_opt(obj, "content"),
                    json_i64_opt(obj, "rating"),
                    json_str_opt(obj, "deleted_at"),
                    json_str_opt(obj, "deleted_by_device"),
                    json_str(obj, "created_at"),
                    json_str(obj, "updated_at"),
                ],
            )?;
            result.reviews += 1;
        }

        // Settings
        for setting in &snapshot.library.settings {
            let obj = setting.as_object().ok_or_else(|| {
                DbError::InvalidOperation("snapshot setting is not an object".into())
            })?;
            tx.execute(
                "INSERT OR IGNORE INTO user_settings
                 (id, key, value, sync_hlc, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    json_str(obj, "id"),
                    json_str(obj, "key"),
                    json_str(obj, "value"),
                    json_str_opt(obj, "sync_hlc"),
                    json_str(obj, "updated_at"),
                ],
            )?;
            result.settings += 1;
        }

        tx.commit()?;
        Ok(result)
    }

    /// Export a table to a vector of JSON objects.
    fn export_table(&self, sql: &str, columns: &[&str]) -> Result<Vec<serde_json::Value>, DbError> {
        let mut stmt = self.db.conn.prepare(sql)?;
        let col_count = columns.len();

        let rows = stmt
            .query_map([], |row| {
                let mut map = serde_json::Map::new();
                for (i, col_name) in columns.iter().enumerate().take(col_count) {
                    let value: rusqlite::types::Value = row.get(i)?;
                    map.insert(
                        col_name.to_string(),
                        match value {
                            rusqlite::types::Value::Null => serde_json::Value::Null,
                            rusqlite::types::Value::Integer(n) => {
                                serde_json::Value::Number(n.into())
                            }
                            rusqlite::types::Value::Real(f) => serde_json::json!(f),
                            rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                            rusqlite::types::Value::Blob(b) => serde_json::Value::String(
                                base64::engine::general_purpose::STANDARD.encode(b),
                            ),
                        },
                    );
                }
                Ok(serde_json::Value::Object(map))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }
}

/// Result of applying a snapshot to the local database.
#[derive(Debug, Default)]
pub struct SnapshotApplyResult {
    pub books: usize,
    pub sessions: usize,
    pub progress: usize,
    pub tags: usize,
    pub notes: usize,
    pub reviews: usize,
    pub settings: usize,
}

// JSON extraction helpers
fn json_str(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn json_str_opt(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(|s| s.to_string())
}

fn json_i64_opt(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    obj.get(key).and_then(|v| v.as_i64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use toku_core::sync::HybridClock;

    fn setup_db() -> Database {
        Database::open_in_memory().expect("in-memory DB")
    }

    fn populate_db(db: &Database) {
        let now = chrono::Utc::now().to_rfc3339();
        let book_id = "01972123-abcd-7000-8000-000000000001";
        let author_id = "01972123-abcd-7000-8000-000000000002";
        let session_id = "01972123-abcd-7000-8000-000000000003";
        let tag_id = "01972123-abcd-7000-8000-000000000004";
        let note_id = "01972123-abcd-7000-8000-000000000005";
        let review_id = "01972123-abcd-7000-8000-000000000006";
        let setting_id = "01972123-abcd-7000-8000-000000000007";

        db.conn
            .execute(
                "INSERT INTO books (id, title, format, status, rating, created_at, updated_at)
                 VALUES (?1, 'Dune', 'physical', 'read', 9, ?2, ?2)",
                params![book_id, now],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO authors (id, name, sort_name) VALUES (?1, 'Frank Herbert', 'Herbert, Frank')",
                params![author_id],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO book_authors (book_id, author_id, role, position)
                 VALUES (?1, ?2, 'author', 0)",
                params![book_id, author_id],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO reading_sessions (id, book_id, started_at, created_at)
                 VALUES (?1, ?2, ?3, ?3)",
                params![session_id, book_id, now],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO tags (id, name, tag_type, created_at) VALUES (?1, 'sci-fi', 'general', ?2)",
                params![tag_id, now],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO book_tags (book_id, tag_id) VALUES (?1, ?2)",
                params![book_id, tag_id],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO notes (id, book_id, content, created_at, updated_at)
                 VALUES (?1, ?2, 'Great book', ?3, ?3)",
                params![note_id, book_id, now],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO reviews (id, book_id, content, rating, created_at, updated_at)
                 VALUES (?1, ?2, 'Masterpiece', 9, ?3, ?3)",
                params![review_id, book_id, now],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO user_settings (id, key, value, updated_at)
                 VALUES (?1, 'theme', 'dark', ?2)",
                params![setting_id, now],
            )
            .unwrap();
    }

    #[test]
    fn export_snapshot_captures_all_entities() {
        let db = setup_db();
        populate_db(&db);

        let repo = SnapshotRepository::new(&db);
        let device = Uuid::now_v7();
        let mut clock = HybridClock::new(&device);
        let hlc = clock.now().to_canonical();

        let snapshot = repo.export_snapshot(device, &hlc).unwrap();

        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.created_by_device, device);
        assert_eq!(snapshot.hlc_at_snapshot, hlc);
        assert_eq!(snapshot.library.books.len(), 1);
        assert_eq!(snapshot.library.book_authors.len(), 1);
        assert_eq!(snapshot.library.sessions.len(), 1);
        assert_eq!(snapshot.library.tags.len(), 1);
        assert_eq!(snapshot.library.book_tags.len(), 1);
        assert_eq!(snapshot.library.notes.len(), 1);
        assert_eq!(snapshot.library.reviews.len(), 1);
        assert_eq!(snapshot.library.settings.len(), 1);

        // Verify book content
        let book = snapshot.library.books[0].as_object().unwrap();
        assert_eq!(book["title"].as_str().unwrap(), "Dune");
        assert_eq!(book["rating"].as_i64().unwrap(), 9);
    }

    #[test]
    fn snapshot_round_trip() {
        let source_db = setup_db();
        populate_db(&source_db);

        let repo = SnapshotRepository::new(&source_db);
        let device = Uuid::now_v7();
        let mut clock = HybridClock::new(&device);
        let hlc = clock.now().to_canonical();

        let snapshot = repo.export_snapshot(device, &hlc).unwrap();

        // Apply to a fresh database
        let target_db = setup_db();
        let target_repo = SnapshotRepository::new(&target_db);
        let result = target_repo.apply_snapshot(&snapshot).unwrap();

        assert_eq!(result.books, 1);
        assert_eq!(result.sessions, 1);
        assert_eq!(result.tags, 1);
        assert_eq!(result.notes, 1);
        assert_eq!(result.reviews, 1);
        assert_eq!(result.settings, 1);

        // Verify data survived the round trip
        let title: String = target_db
            .conn
            .query_row("SELECT title FROM books LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "Dune");

        let author_name: String = target_db
            .conn
            .query_row(
                "SELECT a.name FROM authors a
                 JOIN book_authors ba ON ba.author_id = a.id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(author_name, "Frank Herbert");

        let tag_name: String = target_db
            .conn
            .query_row(
                "SELECT t.name FROM tags t
                 JOIN book_tags bt ON bt.tag_id = t.id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_name, "sci-fi");

        let note_content: String = target_db
            .conn
            .query_row("SELECT content FROM notes LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(note_content, "Great book");

        let setting_val: String = target_db
            .conn
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'theme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(setting_val, "dark");
    }

    #[test]
    fn apply_snapshot_is_idempotent() {
        let db = setup_db();
        populate_db(&db);

        let repo = SnapshotRepository::new(&db);
        let device = Uuid::now_v7();
        let mut clock = HybridClock::new(&device);
        let hlc = clock.now().to_canonical();
        let snapshot = repo.export_snapshot(device, &hlc).unwrap();

        let target_db = setup_db();
        let target_repo = SnapshotRepository::new(&target_db);

        // Apply twice — second should not fail or duplicate
        target_repo.apply_snapshot(&snapshot).unwrap();
        target_repo.apply_snapshot(&snapshot).unwrap();

        let count: i64 = target_db
            .conn
            .query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let db = setup_db();
        populate_db(&db);

        let repo = SnapshotRepository::new(&db);
        let device = Uuid::now_v7();
        let mut clock = HybridClock::new(&device);
        let hlc = clock.now().to_canonical();
        let snapshot = repo.export_snapshot(device, &hlc).unwrap();

        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: LibrarySnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.version, snapshot.version);
        assert_eq!(deserialized.library.books.len(), 1);
    }
}
