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

use serde::Serialize;
use toku_core::{Author, Book, ContributorRole};
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
}
