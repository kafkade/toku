mod backfill;
#[cfg(feature = "sqlcipher")]
pub mod crypt_migrate;
mod database;
mod error;
mod library_io;
mod merge;
mod repo;
mod snapshot;
mod sync_repo;

pub use backfill::{BackfillCounts, backfill_sync_ops};
pub use database::Database;
#[cfg(feature = "sqlcipher")]
pub use database::{process_db_key, set_process_db_key};
pub use error::DbError;
pub use library_io::{LibraryIo, RestoreMode, RestoreResult};
pub use merge::MergeEngine;
pub use repo::{
    BookRepository, book_op_fields, progress_op_fields, session_op_fields, tag_op_fields,
};
pub use snapshot::SnapshotRepository;
pub use sync_repo::{ConflictKeep, SyncConflict, SyncRepository};
