//! C FFI bindings for Toku.
//!
//! This crate exposes a minimal C-compatible API for consuming toku-core and toku-db
//! from native frontends (Swift/iOS, C#/Windows, etc.) via cbindgen-generated headers.
//!
//! # Memory Ownership Rules
//!
//! - **Input strings** (`*const c_char`): Caller owns. Must be valid NUL-terminated UTF-8.
//!   Toku copies the data — the caller may free the input after the call returns.
//! - **Output strings** (`*mut c_char`): Toku allocates. Caller must free with
//!   [`toku_free_string`]. Passing output strings to any other deallocator is undefined
//!   behavior. `toku_free_string` accepts null safely.
//! - **Handles** (`*mut TokuDb`): Created by [`toku_open`], destroyed by [`toku_close`].
//!   Must not be used after closing. Not thread-safe — use from one thread at a time.
//!
//! # Error Handling
//!
//! Every function returns a [`TokuStatus`] code. On error, call [`toku_last_error`] from
//! the **same thread** to retrieve a human-readable error message. The error string is
//! valid until the next FFI call on that thread.
//!
//! # Thread Safety
//!
//! `TokuDb` wraps a SQLite connection which is not `Send`. All calls on a given handle
//! must occur on the same thread. The last-error string is thread-local.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::str::FromStr;

use serde::Serialize;
use toku_core::{
    Author, Book, ContributorRole, CurrentlyReadingInput, ReadingStatus, StatsInput, compute_stats,
};
use toku_db::{BookRepository, Database};

// ── Status codes ────────────────────────────────────────────────────────

/// Result codes returned by all FFI functions.
#[repr(C)]
pub enum TokuStatus {
    /// Operation succeeded.
    Ok = 0,
    /// A required pointer argument was null.
    ErrorNullPointer = 1,
    /// An input string was not valid UTF-8.
    ErrorInvalidUtf8 = 2,
    /// The requested resource was not found.
    ErrorNotFound = 3,
    /// A database or I/O error occurred.
    ErrorDb = 4,
    /// A Rust panic was caught at the FFI boundary.
    ErrorPanic = 5,
}

// ── Opaque handle ───────────────────────────────────────────────────────

/// Opaque handle to an open Toku database. Created by `toku_open`, destroyed by
/// `toku_close`. Do not inspect or modify the contents from C/Swift.
pub struct TokuDb {
    db: Database,
}

// ── FFI-specific DTOs ───────────────────────────────────────────────────

/// Book representation for JSON serialization across the FFI boundary.
/// Uses stable string formats rather than Rust enum variant names.
#[derive(Serialize)]
struct FfiBook {
    id: String,
    title: String,
    subtitle: Option<String>,
    status: String,
    rating: Option<i32>,
    page_count: Option<i32>,
    format: String,
    pub_date: Option<String>,
    language: Option<String>,
    authors: Vec<String>,
}

impl FfiBook {
    fn from_book(book: &Book, authors: Vec<String>) -> Self {
        Self {
            id: book.id.to_string(),
            title: book.title.clone(),
            subtitle: book.subtitle.clone(),
            status: book.status.as_str().to_string(),
            rating: book.rating,
            page_count: book.page_count,
            format: book.format.as_str().to_string(),
            pub_date: book.pub_date.clone(),
            language: book.language.clone(),
            authors,
        }
    }
}

// ── Thread-local error storage ──────────────────────────────────────────

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok();
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Retrieve the last error message for the current thread. Returns null if no error
/// has occurred since the last successful call. The returned pointer is valid until
/// the next FFI call on this thread — do not free it.
///
/// # Safety
/// This function accesses thread-local storage and returns a pointer that is valid
/// only until the next FFI call on the same thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

// ── FFI guard macro ─────────────────────────────────────────────────────

/// Execute a closure, catching panics and converting errors to TokuStatus.
fn ffi_guard<F>(f: F) -> TokuStatus
where
    F: FnOnce() -> TokuStatus + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(status) => status,
        Err(_) => {
            set_last_error("internal panic caught at FFI boundary");
            TokuStatus::ErrorPanic
        }
    }
}

// ── Helper: read a C string ─────────────────────────────────────────────

/// Convert a `*const c_char` to `&str`, returning an error status on null or invalid UTF-8.
///
/// # Safety
/// The caller must ensure `ptr` is a valid, NUL-terminated C string.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Result<&'a str, TokuStatus> {
    if ptr.is_null() {
        set_last_error("null pointer passed where string expected");
        return Err(TokuStatus::ErrorNullPointer);
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().map_err(|_| {
        set_last_error("input string is not valid UTF-8");
        TokuStatus::ErrorInvalidUtf8
    })
}

/// Allocate a C string from a Rust string. Returns null on allocation failure.
fn rust_string_to_c(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// ── Public FFI functions ────────────────────────────────────────────────

/// Open (or create) a Toku database at the given file path.
/// On success, writes a handle to `*out`. The caller must eventually call `toku_close`.
///
/// # Safety
/// - `path` must be a valid NUL-terminated UTF-8 string.
/// - `out` must be a valid pointer to a `*mut TokuDb`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_open(path: *const c_char, out: *mut *mut TokuDb) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if out.is_null() {
            set_last_error("out pointer is null");
            return TokuStatus::ErrorNullPointer;
        }

        let path_str = match unsafe { cstr_to_str(path) } {
            Ok(s) => s,
            Err(status) => return status,
        };

        let db = match Database::open(Path::new(path_str)) {
            Ok(db) => db,
            Err(e) => {
                set_last_error(&format!("failed to open database: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let handle = Box::new(TokuDb { db });
        unsafe { *out = Box::into_raw(handle) };
        TokuStatus::Ok
    }))
}

/// Close a Toku database handle and free its resources.
/// After this call, the handle must not be used. Passing null is a safe no-op.
///
/// # Safety
/// `db` must be a handle previously returned by `toku_open`, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_close(db: *mut TokuDb) {
    if !db.is_null() {
        drop(unsafe { Box::from_raw(db) });
    }
}

/// Add a book with a title and optional author. On success, writes the new book's
/// UUID string to `*out_id`. The caller must free `*out_id` with `toku_free_string`.
///
/// Pass null for `author` to add a book without an author.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `title` must be a valid NUL-terminated UTF-8 string.
/// - `author` may be null, or a valid NUL-terminated UTF-8 string.
/// - `out_id` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_add_book(
    db: *mut TokuDb,
    title: *const c_char,
    author: *const c_char,
    out_id: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_id.is_null() {
            set_last_error("null db or out_id pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let title_str = match unsafe { cstr_to_str(title) } {
            Ok(s) => s,
            Err(status) => return status,
        };

        let author_str = if author.is_null() {
            None
        } else {
            match unsafe { cstr_to_str(author) } {
                Ok("") => None,
                Ok(s) => Some(s),
                Err(status) => return status,
            }
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);
        let book = Book::new(title_str);

        if let Err(e) = repo.create_book(&book) {
            set_last_error(&format!("failed to create book: {e}"));
            return TokuStatus::ErrorDb;
        }

        if let Some(name) = author_str {
            let a = Author::new(name);
            if let Err(e) = repo.add_book_author(&a, &book.id, ContributorRole::Author, 0) {
                set_last_error(&format!("failed to add author: {e}"));
                return TokuStatus::ErrorDb;
            }
        }

        let id_str = rust_string_to_c(&book.id.to_string());
        unsafe { *out_id = id_str };
        TokuStatus::Ok
    }))
}

/// List all books as a JSON array string. On success, writes the JSON string to
/// `*out_json`. The caller must free `*out_json` with `toku_free_string`.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_list_books(
    db: *mut TokuDb,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let books = match repo.list_books() {
            Ok(b) => b,
            Err(e) => {
                set_last_error(&format!("failed to list books: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let ffi_books: Vec<FfiBook> = books
            .iter()
            .map(|b| {
                let authors = repo
                    .get_book_authors(&b.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(a, _)| a.name)
                    .collect();
                FfiBook::from_book(b, authors)
            })
            .collect();

        let json = match serde_json::to_string(&ffi_books) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize books: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// Get a single book by UUID as a JSON string. On success, writes the JSON string to
/// `*out_json`. Returns `ErrorNotFound` if no book matches the ID.
/// The caller must free `*out_json` with `toku_free_string`.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `id` must be a valid NUL-terminated UUID string.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_get_book(
    db: *mut TokuDb,
    id: *const c_char,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let id_str = match unsafe { cstr_to_str(id) } {
            Ok(s) => s,
            Err(status) => return status,
        };

        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => {
                set_last_error("invalid UUID format");
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let book = match repo.get_book(&uuid) {
            Ok(b) => b,
            Err(_) => {
                set_last_error("book not found");
                return TokuStatus::ErrorNotFound;
            }
        };

        let authors = repo
            .get_book_authors(&book.id)
            .unwrap_or_default()
            .into_iter()
            .map(|(a, _)| a.name)
            .collect();

        let ffi_book = FfiBook::from_book(&book, authors);

        let json = match serde_json::to_string(&ffi_book) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize book: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

// ── FFI-specific DTOs (tags, shelves, import) ───────────────────────────

#[derive(Serialize)]
struct FfiTag {
    name: String,
    tag_type: String,
    count: i64,
}

#[derive(Serialize)]
struct FfiShelf {
    name: String,
    is_smart: bool,
    book_count: usize,
}

#[derive(Serialize)]
struct FfiImportReport {
    total_rows: usize,
    imported: usize,
    skipped: usize,
    updated: usize,
    errors: usize,
}

// ── Additional public FFI functions ─────────────────────────────────────

/// Delete a book by UUID. Returns `ErrorNotFound` if no book matches.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `id` must be a valid NUL-terminated UUID string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_delete_book(db: *mut TokuDb, id: *const c_char) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() {
            set_last_error("null db pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let id_str = match unsafe { cstr_to_str(id) } {
            Ok(s) => s,
            Err(status) => return status,
        };

        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => {
                set_last_error("invalid UUID format");
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        match repo.delete_book(&uuid) {
            Ok(true) => TokuStatus::Ok,
            Ok(false) => {
                set_last_error("book not found");
                TokuStatus::ErrorNotFound
            }
            Err(e) => {
                set_last_error(&format!("failed to delete book: {e}"));
                TokuStatus::ErrorDb
            }
        }
    }))
}

/// Update a book's reading status. `status` must be one of:
/// `want-to-read`, `reading`, `read`, `on-hold`, `did-not-finish`.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `id` must be a valid NUL-terminated UUID string.
/// - `status` must be a valid NUL-terminated status string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_update_book_status(
    db: *mut TokuDb,
    id: *const c_char,
    status: *const c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() {
            set_last_error("null db pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let id_str = match unsafe { cstr_to_str(id) } {
            Ok(s) => s,
            Err(st) => return st,
        };
        let status_str = match unsafe { cstr_to_str(status) } {
            Ok(s) => s,
            Err(st) => return st,
        };

        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => {
                set_last_error("invalid UUID format");
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        let reading_status = match ReadingStatus::from_str(status_str) {
            Ok(s) => s,
            Err(_) => {
                set_last_error(&format!("invalid status: {status_str}"));
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        match repo.update_book_status(&uuid, reading_status) {
            Ok(true) => TokuStatus::Ok,
            Ok(false) => {
                set_last_error("book not found");
                TokuStatus::ErrorNotFound
            }
            Err(e) => {
                set_last_error(&format!("failed to update status: {e}"));
                TokuStatus::ErrorDb
            }
        }
    }))
}

/// Update a book's rating (0–10 scale). Pass -1 to clear the rating.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `id` must be a valid NUL-terminated UUID string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_update_book_rating(
    db: *mut TokuDb,
    id: *const c_char,
    rating: i32,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() {
            set_last_error("null db pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let id_str = match unsafe { cstr_to_str(id) } {
            Ok(s) => s,
            Err(st) => return st,
        };

        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => {
                set_last_error("invalid UUID format");
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        if !(-1..=10).contains(&rating) {
            set_last_error("rating must be -1 (clear) or 0–10");
            return TokuStatus::ErrorInvalidUtf8;
        }

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        // -1 means clear — update to 0 and let the app interpret
        let actual_rating = if rating == -1 { 0 } else { rating };

        match repo.update_book_rating(&uuid, actual_rating) {
            Ok(true) => TokuStatus::Ok,
            Ok(false) => {
                set_last_error("book not found");
                TokuStatus::ErrorNotFound
            }
            Err(e) => {
                set_last_error(&format!("failed to update rating: {e}"));
                TokuStatus::ErrorDb
            }
        }
    }))
}

/// Search books using full-text search. Results are returned as a JSON array.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `query` must be a valid NUL-terminated UTF-8 string.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_search_books(
    db: *mut TokuDb,
    query: *const c_char,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let query_str = match unsafe { cstr_to_str(query) } {
            Ok(s) => s,
            Err(st) => return st,
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let books = match repo.search_books(query_str) {
            Ok(b) => b,
            Err(e) => {
                set_last_error(&format!("search failed: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let ffi_books: Vec<FfiBook> = books
            .iter()
            .map(|b| {
                let authors = repo
                    .get_book_authors(&b.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(a, _)| a.name)
                    .collect();
                FfiBook::from_book(b, authors)
            })
            .collect();

        let json = match serde_json::to_string(&ffi_books) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize results: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// Get reading statistics as a JSON object. Pass `year = 0` for all-time stats,
/// or a specific year (e.g. 2025) for year-scoped stats.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_get_stats(
    db: *mut TokuDb,
    year: i32,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);
        let year_opt = if year == 0 { None } else { Some(year) };

        let stats = match gather_stats_for_ffi(&repo, year_opt) {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("failed to compute stats: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let json = match serde_json::to_string(&stats) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize stats: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// List all tags with their counts as a JSON array.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_list_tags(db: *mut TokuDb, out_json: *mut *mut c_char) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let tags = match repo.list_tags_with_counts() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(&format!("failed to list tags: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let ffi_tags: Vec<FfiTag> = tags
            .into_iter()
            .map(|(tag, count)| FfiTag {
                name: tag.name,
                tag_type: tag.tag_type.as_str().to_string(),
                count,
            })
            .collect();

        let json = match serde_json::to_string(&ffi_tags) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize tags: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// Get tags for a specific book as a JSON array.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `id` must be a valid NUL-terminated UUID string.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_get_book_tags(
    db: *mut TokuDb,
    id: *const c_char,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let id_str = match unsafe { cstr_to_str(id) } {
            Ok(s) => s,
            Err(st) => return st,
        };

        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => {
                set_last_error("invalid UUID format");
                return TokuStatus::ErrorInvalidUtf8;
            }
        };

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let tags = match repo.get_book_tags(&uuid) {
            Ok(t) => t,
            Err(e) => {
                set_last_error(&format!("failed to get book tags: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let ffi_tags: Vec<FfiTag> = tags
            .into_iter()
            .map(|tag| FfiTag {
                name: tag.name,
                tag_type: tag.tag_type.as_str().to_string(),
                count: 0,
            })
            .collect();

        let json = match serde_json::to_string(&ffi_tags) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize tags: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// List all shelves as a JSON array.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_list_shelves(
    db: *mut TokuDb,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let handle = unsafe { &*db };
        let repo = BookRepository::new(&handle.db);

        let shelves = match repo.list_shelves() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("failed to list shelves: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        let ffi_shelves: Vec<FfiShelf> = shelves
            .into_iter()
            .map(|s| {
                let count = repo
                    .list_books_in_shelf(&s.name)
                    .map(|b| b.len())
                    .unwrap_or(0);
                FfiShelf {
                    name: s.name,
                    is_smart: s.is_smart,
                    book_count: count,
                }
            })
            .collect();

        let json = match serde_json::to_string(&ffi_shelves) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize shelves: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

/// Import books from a Goodreads CSV export. Returns an import report as JSON.
/// Set `dry_run` to true to preview without modifying the database.
///
/// # Safety
/// - `db` must be a valid handle from `toku_open`.
/// - `csv_path` must be a valid NUL-terminated file path.
/// - `out_json` must be a valid pointer to a `*mut c_char`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_import_goodreads(
    db: *mut TokuDb,
    csv_path: *const c_char,
    dry_run: bool,
    out_json: *mut *mut c_char,
) -> TokuStatus {
    ffi_guard(AssertUnwindSafe(|| {
        clear_last_error();

        if db.is_null() || out_json.is_null() {
            set_last_error("null db or out_json pointer");
            return TokuStatus::ErrorNullPointer;
        }

        let path_str = match unsafe { cstr_to_str(csv_path) } {
            Ok(s) => s,
            Err(st) => return st,
        };

        let handle = unsafe { &*db };
        let opts = toku_import::GoodreadsImportOptions { dry_run };

        let report =
            match toku_import::import_goodreads(&handle.db, Path::new(path_str), &opts, None) {
                Ok(r) => r,
                Err(e) => {
                    set_last_error(&format!("import failed: {e}"));
                    return TokuStatus::ErrorDb;
                }
            };

        let ffi_report = FfiImportReport {
            total_rows: report.total_rows,
            imported: report.imported,
            skipped: report.skipped,
            updated: report.updated,
            errors: report.errors,
        };

        let json = match serde_json::to_string(&ffi_report) {
            Ok(j) => j,
            Err(e) => {
                set_last_error(&format!("failed to serialize report: {e}"));
                return TokuStatus::ErrorDb;
            }
        };

        unsafe { *out_json = rust_string_to_c(&json) };
        TokuStatus::Ok
    }))
}

// ── Stats helper ────────────────────────────────────────────────────────

/// Gather data and compute reading statistics — mirrors the web dashboard's logic.
fn gather_stats_for_ffi(
    repo: &BookRepository<'_>,
    year: Option<i32>,
) -> Result<toku_core::ReadingStats, String> {
    let books = repo.list_books().map_err(|e| e.to_string())?;
    let book_ids: Vec<String> = books.iter().map(|b| b.id.to_string()).collect();

    let sessions = match year {
        Some(y) => repo.list_reading_sessions_in_year(y),
        None => repo.list_reading_sessions(),
    }
    .map_err(|e| e.to_string())?;

    let currently_reading_details = repo
        .get_currently_reading_details()
        .map_err(|e| e.to_string())?;
    let currently_reading_input: Vec<CurrentlyReadingInput> = currently_reading_details
        .into_iter()
        .map(|(book, progress, authors)| {
            let author_name = authors
                .into_iter()
                .map(|(a, _)| a.name)
                .collect::<Vec<_>>()
                .join(", ");
            CurrentlyReadingInput {
                title: book.title,
                author: author_name,
                page_count: book.page_count,
                latest_progress: progress,
            }
        })
        .collect();

    let tag_counts = repo.list_tag_counts().map_err(|e| e.to_string())?;
    let author_counts = repo.list_author_book_counts().map_err(|e| e.to_string())?;

    let activity_dates = match year {
        Some(y) => repo.list_activity_dates_in_year(y),
        None => repo.list_activity_dates(),
    }
    .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now();
    let today = chrono::Local::now().date_naive();
    let mood_tag_data = repo
        .get_mood_tags_for_books(&book_ids)
        .map_err(|e| e.to_string())?;

    let stats = compute_stats(StatsInput {
        books: &books,
        sessions: &sessions,
        currently_reading: &currently_reading_input,
        tag_counts: &tag_counts,
        author_counts: &author_counts,
        activity_dates: &activity_dates,
        now,
        today,
        mood_tag_data: &mood_tag_data,
    });

    Ok(stats)
}

/// Free a string that was allocated by a `toku_*` function. Passing null is a safe no-op.
///
/// # Safety
/// `s` must be a string pointer previously returned by a `toku_*` function, or null.
/// Do not pass strings from other allocators.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toku_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn open_memory_db() -> *mut TokuDb {
        let db = Database::open_in_memory().unwrap();
        Box::into_raw(Box::new(TokuDb { db }))
    }

    #[test]
    fn roundtrip_add_list_get() {
        let db = open_memory_db();

        // Add a book
        let title = CString::new("Dune").unwrap();
        let author = CString::new("Frank Herbert").unwrap();
        let mut out_id: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_add_book(db, title.as_ptr(), author.as_ptr(), &mut out_id) };
        assert!(matches!(status, TokuStatus::Ok));
        assert!(!out_id.is_null());

        let id_str = unsafe { CStr::from_ptr(out_id) }.to_str().unwrap();
        assert_eq!(id_str.len(), 36); // UUID format
        let id_copy = id_str.to_string();

        unsafe { toku_free_string(out_id) };

        // List books
        let mut out_json: *mut c_char = std::ptr::null_mut();
        let status = unsafe { toku_list_books(db, &mut out_json) };
        assert!(matches!(status, TokuStatus::Ok));
        assert!(!out_json.is_null());

        let json = unsafe { CStr::from_ptr(out_json) }.to_str().unwrap();
        let books: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0]["title"], "Dune");
        assert_eq!(books[0]["authors"][0], "Frank Herbert");
        assert_eq!(books[0]["status"], "want-to-read");

        unsafe { toku_free_string(out_json) };

        // Get book by ID
        let id_cstr = CString::new(id_copy).unwrap();
        let mut out_book: *mut c_char = std::ptr::null_mut();
        let status = unsafe { toku_get_book(db, id_cstr.as_ptr(), &mut out_book) };
        assert!(matches!(status, TokuStatus::Ok));
        assert!(!out_book.is_null());

        let book_json = unsafe { CStr::from_ptr(out_book) }.to_str().unwrap();
        let book: serde_json::Value = serde_json::from_str(book_json).unwrap();
        assert_eq!(book["title"], "Dune");
        assert_eq!(book["format"], "physical");

        unsafe { toku_free_string(out_book) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn add_book_without_author() {
        let db = open_memory_db();
        let title = CString::new("Untitled").unwrap();
        let mut out_id: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_add_book(db, title.as_ptr(), std::ptr::null(), &mut out_id) };
        assert!(matches!(status, TokuStatus::Ok));
        assert!(!out_id.is_null());

        // Verify no authors in JSON
        let mut out_json: *mut c_char = std::ptr::null_mut();
        unsafe { toku_list_books(db, &mut out_json) };
        let json = unsafe { CStr::from_ptr(out_json) }.to_str().unwrap();
        let books: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(books[0]["authors"].as_array().unwrap().is_empty());

        unsafe { toku_free_string(out_id) };
        unsafe { toku_free_string(out_json) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn utf8_roundtrip() {
        let db = open_memory_db();
        let title = CString::new("Gödelのプルーフ — «Доказательство»").unwrap();
        let author = CString::new("José García Márquez").unwrap();
        let mut out_id: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_add_book(db, title.as_ptr(), author.as_ptr(), &mut out_id) };
        assert!(matches!(status, TokuStatus::Ok));

        let mut out_json: *mut c_char = std::ptr::null_mut();
        unsafe { toku_list_books(db, &mut out_json) };
        let json = unsafe { CStr::from_ptr(out_json) }.to_str().unwrap();
        let books: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(books[0]["title"], "Gödelのプルーフ — «Доказательство»");
        assert_eq!(books[0]["authors"][0], "José García Márquez");

        unsafe { toku_free_string(out_id) };
        unsafe { toku_free_string(out_json) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn null_pointer_safety() {
        // All functions should return ErrorNullPointer for null db handle
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_list_books(std::ptr::null_mut(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));
        assert!(out.is_null());

        let title = CString::new("Test").unwrap();
        let status = unsafe {
            toku_add_book(
                std::ptr::null_mut(),
                title.as_ptr(),
                std::ptr::null(),
                &mut out,
            )
        };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        let id = CString::new("00000000-0000-0000-0000-000000000000").unwrap();
        let status = unsafe { toku_get_book(std::ptr::null_mut(), id.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // Null title should fail
        let db = open_memory_db();
        let status = unsafe { toku_add_book(db, std::ptr::null(), std::ptr::null(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // toku_close with null is safe
        unsafe { toku_close(std::ptr::null_mut()) };

        // toku_free_string with null is safe
        unsafe { toku_free_string(std::ptr::null_mut()) };

        unsafe { toku_close(db) };
    }

    #[test]
    fn get_book_not_found() {
        let db = open_memory_db();
        let id = CString::new("01961234-5678-7000-8000-000000000000").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_get_book(db, id.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNotFound));
        assert!(out.is_null());

        // Check last error message
        let err = unsafe { toku_last_error() };
        assert!(!err.is_null());
        let msg = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert!(msg.contains("not found"));

        unsafe { toku_close(db) };
    }

    #[test]
    fn invalid_uuid_format() {
        let db = open_memory_db();
        let id = CString::new("not-a-uuid").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_get_book(db, id.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorInvalidUtf8));

        unsafe { toku_close(db) };
    }

    #[test]
    fn last_error_cleared_on_success() {
        let db = open_memory_db();

        // Cause an error
        let id = CString::new("not-a-uuid").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        unsafe { toku_get_book(db, id.as_ptr(), &mut out) };

        let err = unsafe { toku_last_error() };
        assert!(!err.is_null());

        // Successful call clears error
        let title = CString::new("Test").unwrap();
        unsafe { toku_add_book(db, title.as_ptr(), std::ptr::null(), &mut out) };

        let err = unsafe { toku_last_error() };
        assert!(err.is_null());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    // Helper: add a book and return its UUID string
    fn add_test_book(db: *mut TokuDb, title: &str, author: Option<&str>) -> String {
        let title_c = CString::new(title).unwrap();
        let author_c = author.map(|a| CString::new(a).unwrap());
        let mut out_id: *mut c_char = std::ptr::null_mut();

        let status = unsafe {
            toku_add_book(
                db,
                title_c.as_ptr(),
                author_c.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
                &mut out_id,
            )
        };
        assert!(matches!(status, TokuStatus::Ok));

        let id = unsafe { CStr::from_ptr(out_id) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { toku_free_string(out_id) };
        id
    }

    #[test]
    fn delete_book_success() {
        let db = open_memory_db();
        let id = add_test_book(db, "To Delete", Some("Author"));
        let id_c = CString::new(id).unwrap();

        let status = unsafe { toku_delete_book(db, id_c.as_ptr()) };
        assert!(matches!(status, TokuStatus::Ok));

        // Verify it's gone
        let mut out: *mut c_char = std::ptr::null_mut();
        unsafe { toku_list_books(db, &mut out) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let books: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(books.is_empty());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn delete_book_not_found() {
        let db = open_memory_db();
        let id = CString::new("01961234-5678-7000-8000-000000000000").unwrap();

        let status = unsafe { toku_delete_book(db, id.as_ptr()) };
        assert!(matches!(status, TokuStatus::ErrorNotFound));

        unsafe { toku_close(db) };
    }

    #[test]
    fn update_status_success() {
        let db = open_memory_db();
        let id = add_test_book(db, "Status Test", None);
        let id_c = CString::new(id.clone()).unwrap();
        let status_c = CString::new("reading").unwrap();

        let result = unsafe { toku_update_book_status(db, id_c.as_ptr(), status_c.as_ptr()) };
        assert!(matches!(result, TokuStatus::Ok));

        // Verify the status changed
        let mut out: *mut c_char = std::ptr::null_mut();
        let id_c2 = CString::new(id).unwrap();
        unsafe { toku_get_book(db, id_c2.as_ptr(), &mut out) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let book: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(book["status"], "reading");

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn update_status_invalid() {
        let db = open_memory_db();
        let id = add_test_book(db, "Bad Status", None);
        let id_c = CString::new(id).unwrap();
        let status_c = CString::new("invalid-status").unwrap();

        let result = unsafe { toku_update_book_status(db, id_c.as_ptr(), status_c.as_ptr()) };
        assert!(matches!(result, TokuStatus::ErrorInvalidUtf8));

        unsafe { toku_close(db) };
    }

    #[test]
    fn update_rating_success() {
        let db = open_memory_db();
        let id = add_test_book(db, "Rating Test", None);
        let id_c = CString::new(id.clone()).unwrap();

        let result = unsafe { toku_update_book_rating(db, id_c.as_ptr(), 8) };
        assert!(matches!(result, TokuStatus::Ok));

        // Verify the rating changed
        let mut out: *mut c_char = std::ptr::null_mut();
        let id_c2 = CString::new(id).unwrap();
        unsafe { toku_get_book(db, id_c2.as_ptr(), &mut out) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let book: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(book["rating"], 8);

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn update_rating_out_of_range() {
        let db = open_memory_db();
        let id = add_test_book(db, "Bad Rating", None);
        let id_c = CString::new(id).unwrap();

        let result = unsafe { toku_update_book_rating(db, id_c.as_ptr(), 11) };
        assert!(matches!(result, TokuStatus::ErrorInvalidUtf8));

        let result = unsafe { toku_update_book_rating(db, id_c.as_ptr(), -2) };
        assert!(matches!(result, TokuStatus::ErrorInvalidUtf8));

        unsafe { toku_close(db) };
    }

    #[test]
    fn search_books_returns_results() {
        let db = open_memory_db();
        add_test_book(db, "Dune Messiah", Some("Frank Herbert"));
        add_test_book(db, "Foundation", Some("Isaac Asimov"));

        let query = CString::new("Dune").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_search_books(db, query.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::Ok));

        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let books: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0]["title"], "Dune Messiah");

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn get_stats_returns_json() {
        let db = open_memory_db();
        add_test_book(db, "Stats Book", None);

        let mut out: *mut c_char = std::ptr::null_mut();
        let status = unsafe { toku_get_stats(db, 0, &mut out) };
        assert!(matches!(status, TokuStatus::Ok));

        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let stats: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(stats.get("total_books").is_some());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn list_tags_empty() {
        let db = open_memory_db();
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_list_tags(db, &mut out) };
        assert!(matches!(status, TokuStatus::Ok));

        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let tags: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(tags.is_empty());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn list_shelves_empty() {
        let db = open_memory_db();
        let mut out: *mut c_char = std::ptr::null_mut();

        let status = unsafe { toku_list_shelves(db, &mut out) };
        assert!(matches!(status, TokuStatus::Ok));

        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let shelves: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(shelves.is_empty());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn get_book_tags_not_found() {
        let db = open_memory_db();
        let id = CString::new("01961234-5678-7000-8000-000000000000").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();

        // Tags for non-existent book returns empty array (not error)
        let status = unsafe { toku_get_book_tags(db, id.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::Ok));

        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        let tags: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(tags.is_empty());

        unsafe { toku_free_string(out) };
        unsafe { toku_close(db) };
    }

    #[test]
    fn null_pointer_new_functions() {
        let mut out: *mut c_char = std::ptr::null_mut();

        // delete_book with null db
        let id = CString::new("01961234-5678-7000-8000-000000000000").unwrap();
        let status = unsafe { toku_delete_book(std::ptr::null_mut(), id.as_ptr()) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // update_status with null db
        let status_c = CString::new("reading").unwrap();
        let status = unsafe {
            toku_update_book_status(std::ptr::null_mut(), id.as_ptr(), status_c.as_ptr())
        };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // update_rating with null db
        let status = unsafe { toku_update_book_rating(std::ptr::null_mut(), id.as_ptr(), 5) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // search_books with null db
        let query = CString::new("test").unwrap();
        let status = unsafe { toku_search_books(std::ptr::null_mut(), query.as_ptr(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // get_stats with null db
        let status = unsafe { toku_get_stats(std::ptr::null_mut(), 0, &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // list_tags with null db
        let status = unsafe { toku_list_tags(std::ptr::null_mut(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));

        // list_shelves with null db
        let status = unsafe { toku_list_shelves(std::ptr::null_mut(), &mut out) };
        assert!(matches!(status, TokuStatus::ErrorNullPointer));
    }
}
