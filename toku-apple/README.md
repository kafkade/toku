# toku-apple

Native Apple apps for Toku, built with SwiftUI and the Rust FFI layer.

- **macOS app** — full-featured desktop app with sidebar navigation, sortable
  table, cover grid, statistics dashboard, and Goodreads CSV import.
- **iOS app** — iPhone and iPad app with touch-optimized library grid, barcode
  scanner, quick progress updates, and adaptive navigation.

Both apps share the **TokuKit** Swift package for FFI, models, and view models.

## Architecture

```text
SwiftUI Views → Swift ViewModels → TokuFFI wrapper → toku-ffi (C ABI) → toku-core / toku-db
```

### Components

| Component | Description |
|---|---|
| `TokuKit/` | Swift Package with FFI wrapper, models, ViewModels, and shared UI components |
| `Toku/` | macOS SwiftUI app target |
| `TokuiOS/` | iOS SwiftUI app target (iPhone + iPad) |
| `TokuKit/Sources/CTokuFFI/` | C module map pointing to the `toku.h` header |
| `TokuKit/Sources/TokuKitUI/` | Shared SwiftUI components (StarRatingView, FlowLayout, etc.) |

## Prerequisites

### macOS app

- macOS 14 (Sonoma) or later
- Xcode 15.4 or later
- Rust toolchain (`rustup` with `aarch64-apple-darwin` target)

### iOS app

- iOS 17.0 or later
- Xcode 15.4 or later
- Rust toolchain with iOS targets:

  ```bash
  rustup target add aarch64-apple-ios         # Device
  rustup target add aarch64-apple-ios-sim     # Apple Silicon simulator
  ```

## Build steps

### 1. Build the Rust FFI library

#### macOS

```bash
# From the repository root
cargo build --release -p toku-ffi

# The static library will be at:
# target/release/libtoku_ffi.a
```

#### iOS

```bash
# Device binary
cargo build --release -p toku-ffi --target aarch64-apple-ios

# Simulator binary (Apple Silicon)
cargo build --release -p toku-ffi --target aarch64-apple-ios-sim

# The static libraries will be at:
# target/aarch64-apple-ios/release/libtoku_ffi.a
# target/aarch64-apple-ios-sim/release/libtoku_ffi.a
```

### 2. Copy the header

The header is auto-generated during build. Copy it to the Swift package:

```bash
cp crates/toku-ffi/toku.h toku-apple/TokuKit/Sources/CTokuFFI/toku.h
```

### 3. Configure the Xcode project

Since this is a source-only delivery (no `.xcodeproj` checked in), create the
project in Xcode:

#### macOS target

1. **File → New → Project → macOS → App** (SwiftUI, Swift)
2. Product name: `Toku`, bundle ID: `dev.toku.app`
3. Set deployment target to **macOS 14.0**
4. Add `TokuKit` and `TokuKitUI` as local Swift Package dependencies:
   **File → Add Package → Add Local → select `toku-apple/TokuKit/`**
5. Add the existing Swift source files from `Toku/` to the app target
6. In **Build Settings**:
   - **Library Search Paths**: add `$(PROJECT_DIR)/../target/release`
   - **Other Linker Flags**: add `-ltoku_ffi -lsqlite3 -framework Security`
7. Build and run (⌘R)

#### iOS target

1. **File → New Target → iOS → App** (SwiftUI, Swift) — or add to the existing
   project
2. Product name: `TokuiOS`, bundle ID: `dev.toku.ios`
3. Set deployment target to **iOS 17.0**
4. Supported destinations: **iPhone** and **iPad**
5. Add `TokuKit` and `TokuKitUI` as dependencies of the iOS target:
   **Target → General → Frameworks, Libraries, and Embedded Content**
6. Add the existing Swift source files from `TokuiOS/` to the iOS target
7. In **Build Settings**:
   - **Library Search Paths**:
     - Device: `$(PROJECT_DIR)/../target/aarch64-apple-ios/release`
     - Simulator: `$(PROJECT_DIR)/../target/aarch64-apple-ios-sim/release`
   - **Other Linker Flags**: add `-ltoku_ffi -lsqlite3 -framework Security`
8. Copy `TokuiOS/Info.plist` to the target's info plist
9. Build and run on device or simulator (⌘R)

## Features

### macOS app

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

### iOS app

| Feature | Status |
|---|---|
| Library grid with cover cards | ✅ |
| Search with full-text search | ✅ |
| Book detail with metadata and tags | ✅ |
| Quick progress update (≤3 taps) | ✅ |
| Barcode scanner (ISBN-13 via camera) | ✅ |
| Manual ISBN entry fallback | ✅ |
| Statistics glance with Swift Charts | ✅ |
| Goodreads CSV import (file picker) | ✅ |
| iPad NavigationSplitView with sidebar | ✅ |
| iPhone TabView navigation | ✅ |
| Status filter chips | ✅ |
| Pull-to-refresh | ✅ |
| Dark mode (system preference) | ✅ |
| Fully offline | ✅ |

## Data storage

### macOS

```text
~/Library/Application Support/dev.toku.app/library.db
```

### iOS

```text
<App Container>/Library/Application Support/dev.toku.ios/library.db
```

## Known limitations

### FFI gaps (to be addressed)

- **Reading progress logging**: `toku-db` has `log_progress()` but `toku-ffi`
  does not yet expose it. The progress update sheet currently updates reading
  status only. A `toku_log_progress` FFI function is needed.
- **ISBN-based book addition**: The barcode scanner captures ISBN-13 codes but
  the current `toku_add_book` FFI function only accepts title/author. An
  ISBN-aware add function or metadata fetch through `toku-meta` FFI is needed.

### What cannot be validated on Windows

- Swift compilation and Xcode project setup
- Linking against `libtoku_ffi.a` (any platform)
- SwiftUI rendering and interaction
- macOS-specific APIs (NSOpenPanel, drag-and-drop)
- iOS-specific APIs (AVFoundation barcode scanning, UIDocumentPicker)
- iOS simulator and device testing

All Rust FFI code is validated by the `toku-ffi` test suite which runs on all
platforms in CI.
