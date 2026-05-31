# ADR-009: Native App Architecture — SwiftUI + FFI for Apple, Tauri v2 for Windows

**Status**: Accepted
**Date**: 2026-05-31
**Decision**: Use SwiftUI consuming Rust core via C FFI for macOS and iOS apps. Use
Tauri v2 wrapping the `toku-web` server-rendered UI for the Windows desktop app.

## Context

Phase 5 required native apps for macOS, iOS, and Windows. The core library (`toku-core`,
`toku-db`) is written in Rust. The challenge is bridging Rust to native UI frameworks
on each platform while maximizing code reuse and maintaining native UX quality.

Three approaches were evaluated:

1. **SwiftUI + Rust FFI** — Native Apple UI calling Rust via C function exports.
2. **Tauri v2** — Web-based UI in a native window, with Rust backend.
3. **Cross-platform framework** — React Native, Flutter, or Kotlin Multiplatform.

## Decision

### Apple platforms (macOS + iOS): SwiftUI + `toku-ffi`

- The `toku-ffi` crate exports a C API using `#[no_mangle]` and `extern "C"` functions.
- `cbindgen` generates the C header (`toku.h`) from Rust source.
- `TokuKit` (Swift package in `toku-apple/TokuKit/`) wraps the C API in a Swift-native
  interface: `TokuFFI` class with safe Swift types, error handling, and JSON decoding.
- SwiftUI views consume `TokuFFI` methods. The Xcode project builds `toku-ffi` as a
  static library and links it into the Swift app.

**FFI API surface** (9 core functions):

| Function | Purpose |
|----------|---------|
| `toku_open` | Open database, run migrations |
| `toku_list_books` | List all books as JSON |
| `toku_get_book` | Get single book detail as JSON |
| `toku_search` | Full-text search, return JSON results |
| `toku_delete_book` | Delete a book by ID |
| `toku_update_status` | Change reading status |
| `toku_update_rating` | Set book rating |
| `toku_get_stats` | Get statistics as JSON |
| `toku_import_goodreads` | Import Goodreads CSV, return report JSON |

**Threading model**: All `TokuFFI` methods must run on the same thread that opened the
database handle. The Swift layer uses a dedicated serial `DispatchQueue` for all FFI
calls, with `@MainActor` bridging for UI updates.

**Shared UI components**: `TokuKitUI` (SwiftUI package) provides reusable views used
by both macOS and iOS apps: `StarRatingView`, `FlowLayout`, `MetadataRow`, `StatCard`.

### iOS-specific features

- Barcode scanner (ISBN-13 via `AVCaptureSession` + `VNBarcodeObservation`)
- Adaptive navigation: `TabView` on iPhone, `NavigationSplitView` on iPad
- Quick progress update sheet
- Pull-to-refresh on library grid

### macOS-specific features

- Sidebar navigation with `NavigationSplitView`
- Sortable table view (SwiftUI `Table`)
- Book detail inspector pane
- Swift Charts statistics dashboard

### Windows: Tauri v2 + `toku-web`

- The `toku-desktop` crate wraps the `toku-web` server in a Tauri v2 window.
- Tauri starts an embedded Axum server and loads the web UI in a native WebView2 window.
- System tray support with minimize-to-tray.
- No separate Windows-native UI code — the web UI is the Windows UI.

## Rationale

### Why SwiftUI + FFI over Tauri for Apple?

1. **UX quality**: SwiftUI produces genuinely native Apple UI — system fonts, animations,
   navigation patterns, accessibility, Dynamic Type, and Dark Mode work automatically.
   A WebView-based app on macOS/iOS feels foreign.
2. **Platform features**: Barcode scanning (AVFoundation), Swift Charts, system Share
   sheet, Spotlight integration, and Shortcuts are trivial with SwiftUI, difficult or
   impossible through a WebView.
3. **App Store expectations**: iOS App Store review favors native UI. WebView-only apps
   risk rejection under guideline 4.2 (Minimum Functionality).
4. **Performance**: Native rendering with no WebView overhead. Instant launch.

### Why Tauri for Windows over WinUI?

1. **Code reuse**: The web UI (`toku-web`) already exists from Phase 4. Wrapping it in
   Tauri requires minimal Windows-specific code.
2. **Maintenance cost**: Building a WinUI 3 app would require learning a new framework,
   maintaining a separate C#/XAML codebase, and duplicating UI logic. Solo developer
   cannot sustain two native UI stacks plus a web UI.
3. **Quality bar**: Windows users are accustomed to Electron/WebView apps (VS Code,
   Discord, Slack, Notion). The quality bar for a WebView-based Windows app is lower
   than on macOS.
4. **Tauri v2 maturity**: Tauri v2 uses WebView2 (Edge-based), is stable, and provides
   system tray, auto-update, and native window management.

### Why not React Native / Flutter / Kotlin Multiplatform?

1. **React Native**: Requires a JavaScript runtime, adds a Node.js build chain, and the
   Rust FFI bridge is more complex (JSI + Turbo Modules). Doesn't solve the "web UI
   already exists" problem for Windows.
2. **Flutter**: Requires Dart, a separate build system, and the Rust FFI bridge uses
   `dart:ffi`. Flutter on macOS/iOS does not feel native — custom rendering engine
   bypasses system controls.
3. **Kotlin Multiplatform**: Strong for Android but weak on iOS/macOS (still maturing).
   No Windows story. Doesn't leverage existing Rust core.

All three add a new language and build system without meaningfully improving the outcome
over SwiftUI (Apple) + Tauri (Windows).

## Consequences

- **Two UI paradigms**: SwiftUI for Apple and web UI for Windows means UI changes need
  to be made in two places (Swift views + maud templates). The core logic is shared via
  Rust, but presentation is duplicated. This is an accepted trade-off for native quality.
- **FFI maintenance**: Changes to `toku-core` or `toku-db` may require updating `toku-ffi`
  exports, regenerating `toku.h`, and updating `TokuFFI.swift`. This is a manual process.
- **JSON bridge**: FFI functions return JSON strings decoded on the Swift side. This is
  simple and debuggable but adds serialization overhead. For the data volumes involved
  (hundreds of books, not millions), this is negligible.
- **No Android**: This ADR does not address Android. When Android is needed, Kotlin +
  Rust FFI (via JNI or UniFFI) is the expected path — similar to the Swift approach.
- **Tauri v2 dependency**: The Windows app depends on WebView2, which requires Windows 10
  (build 17763+) or Windows 11. Older Windows versions are not supported.
