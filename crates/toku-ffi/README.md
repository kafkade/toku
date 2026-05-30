# toku-ffi

C-compatible FFI bindings for Toku, generated with
[cbindgen](https://github.com/mozilla/cbindgen). Intended for consumption by
native frontends (Swift/iOS, C#/Windows, Kotlin/JNI).

## API surface

| Function | Purpose |
|---|---|
| `toku_open` | Open (or create) a database at a file path |
| `toku_close` | Close a database handle |
| `toku_add_book` | Add a book with title and optional author |
| `toku_list_books` | List all books as a JSON array |
| `toku_get_book` | Get a single book by UUID as JSON |
| `toku_free_string` | Free a string allocated by any `toku_*` function |
| `toku_last_error` | Retrieve the last error message (thread-local) |

## Generated header

`toku.h` is generated at build time by cbindgen. Do not edit manually.

## Memory ownership

- **Input strings** (`const char *`): caller-owned, NUL-terminated UTF-8. Toku
  copies the data — the caller may free after the call returns.
- **Output strings** (`char **`): Toku-allocated. Caller **must** free with
  `toku_free_string`. Do not pass to `free()` or any other deallocator.
- **Handles** (`TokuDb *`): created by `toku_open`, destroyed by `toku_close`.
  Must not be used after closing.

## Error handling

Every function returns a `TokuStatus` enum:

| Code | Name | Meaning |
|---|---|---|
| 0 | `TOKU_STATUS_OK` | Success |
| 1 | `TOKU_STATUS_ERROR_NULL_POINTER` | A required pointer argument was null |
| 2 | `TOKU_STATUS_ERROR_INVALID_UTF8` | Input string is not valid UTF-8 |
| 3 | `TOKU_STATUS_ERROR_NOT_FOUND` | Requested resource not found |
| 4 | `TOKU_STATUS_ERROR_DB` | Database or I/O error |
| 5 | `TOKU_STATUS_ERROR_PANIC` | Rust panic caught at FFI boundary |

On error, call `toku_last_error()` from the **same thread** for a
human-readable message. The returned pointer is valid until the next FFI call
on that thread.

## Thread safety

`TokuDb` wraps a SQLite connection (`rusqlite::Connection`) which is `!Send`.
All calls on a given handle must occur on the same thread.

## Swift usage example

```swift
import Foundation

// Load the dylib or use the static lib linked into your target
var db: OpaquePointer?
let status = toku_open("/path/to/library.db", &db)
guard status == TOKU_STATUS_OK, let db = db else {
    let err = String(cString: toku_last_error())
    fatalError("Failed to open database: \(err)")
}

// Add a book
var bookId: UnsafeMutablePointer<CChar>?
toku_add_book(db, "Dune", "Frank Herbert", &bookId)
if let id = bookId {
    print("Added book: \(String(cString: id))")
    toku_free_string(id)
}

// List all books
var json: UnsafeMutablePointer<CChar>?
toku_list_books(db, &json)
if let j = json {
    print(String(cString: j))
    toku_free_string(j)
}

toku_close(db)
```

## JSON format

Books are serialized with stable string values (not Rust enum variants):

```json
{
  "id": "01961234-5678-7000-8000-000000000000",
  "title": "Dune",
  "subtitle": null,
  "status": "want-to-read",
  "rating": null,
  "page_count": 412,
  "format": "physical",
  "pub_date": "1965",
  "language": "en",
  "authors": ["Frank Herbert"]
}
```

Status values: `want-to-read`, `reading`, `read`, `on-hold`, `did-not-finish`.
Format values: `physical`, `ebook`, `audiobook`.

## What cannot be validated on Windows

- Swift integration test (requires Xcode / Swift toolchain)
- iOS cross-compilation (`cargo build --target aarch64-apple-ios`)
- dylib loading in a real iOS/macOS app

These are manual validation steps for macOS development.
