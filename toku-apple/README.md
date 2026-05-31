# toku-apple

Native macOS app for Toku, built with SwiftUI and the Rust FFI layer.

## Architecture

```text
SwiftUI Views → Swift ViewModels → TokuFFI wrapper → toku-ffi (C ABI) → toku-core / toku-db
```

### Components

| Component | Description |
|---|---|
| `TokuKit/` | Swift Package with FFI wrapper, models, and ViewModels (shared with future iOS app) |
| `Toku/` | macOS SwiftUI app target |
| `TokuKit/Sources/CTokuFFI/` | C module map pointing to the `toku.h` header |

## Prerequisites

- macOS 14 (Sonoma) or later
- Xcode 15.4 or later
- Rust toolchain (`rustup` with `aarch64-apple-darwin` target)

## Build steps

### 1. Build the Rust FFI library

```bash
# From the repository root
cargo build --release -p toku-ffi

# The static library will be at:
# target/release/libtoku_ffi.a
```

### 2. Copy the header

The header is auto-generated during build. Copy it to the Swift package:

```bash
cp crates/toku-ffi/toku.h toku-apple/TokuKit/Sources/CTokuFFI/toku.h
```

### 3. Open in Xcode

```bash
cd toku-apple
open Toku.xcodeproj   # or create from Xcode: File > New > Project
```

### 4. Configure the Xcode project

Since this is a source-only delivery (no `.xcodeproj` checked in), create the
project in Xcode:

1. **File → New → Project → macOS → App** (SwiftUI, Swift)
2. Product name: `Toku`, bundle ID: `dev.toku.app`
3. Set deployment target to **macOS 14.0**
4. Add `TokuKit` as a local Swift Package dependency:
   **File → Add Package → Add Local → select `toku-apple/TokuKit/`**
5. Add the existing Swift source files from `Toku/` to the app target
6. In **Build Settings**:
   - **Library Search Paths**: add `$(PROJECT_DIR)/../target/release`
   - **Other Linker Flags**: add `-ltoku_ffi -lsqlite3 -framework Security`
7. Build and run (⌘R)

## Features

| Feature | Status |
|---|---|
| Sidebar navigation (Library, Grid, Stats, Import) | ✅ |
| Multi-column table with sortable columns | ✅ |
| Cover grid view | ✅ |
| Book detail inspector panel | ✅ |
| Statistics dashboard (Swift Charts) | ✅ |
| Full-text search (toolbar) | ✅ |
| Goodreads CSV import with drag-and-drop | ✅ |
| Keyboard shortcuts (⌘N, ⌘I, ⌘R, ⌘F) | ✅ |
| Standard macOS menu bar | ✅ |
| Dark mode (system preference) | ✅ |
| Fully offline | ✅ |

## Data storage

The database is stored in Application Support:

```text
~/Library/Application Support/dev.toku.app/library.db
```

## What cannot be validated on Windows

- Swift compilation and Xcode project setup
- Linking against `libtoku_ffi.a`
- SwiftUI rendering and interaction
- macOS-specific APIs (NSOpenPanel, drag-and-drop)

All Rust FFI code is validated by the `toku-ffi` test suite which runs on all
platforms in CI.
