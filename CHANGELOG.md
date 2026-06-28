# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- The published multi-arch `toku-sync` Docker image now builds successfully for
  `linux/arm64` (e.g. Raspberry Pi), completing the ARM build fix from 0.3.1 that
  still failed while compiling bundled SQLite

## [0.3.0] - 2026-06-27

### Added

- Account-based sync CLI: `toku sync signup`, `toku sync login`, and
  `toku sync enroll` bring the 1Password-style two-secret flow to the command
  line. `signup` creates an account, generates the device Secret Key, renders the
  Emergency Kit once (printed, or written with `--kit-out` as `.pdf`/`.html`/text),
  and enrolls the first device as admin in one step. `login` re-authenticates an
  already-enrolled device; `enroll` joins an existing account from a new device,
  recovering the shared library data key the zero-knowledge way (SRP login → fetch
  the wrapped key bundle → unwrap the same `SyncKey` locally). The account password
  and Secret Key are read with non-echoing prompts and never appear in argv or
  shell history, and the Secret Key is never written to disk. `toku sync devices`
  now lists your account's devices across the account when you are logged in
- Account-aware `toku-sync-client`: account signup/challenge/verify, device
  enrollment, account-scoped device listing/removal, and account key-bundle
  retrieval, with a separate account (user) session stored alongside the
  per-device sync token

- Account **Secret Key** + **Emergency Kit** for zero-knowledge sync (1Password-style
  two-secret model). A high-entropy 128-bit Secret Key is generated on-device, formatted
  for transcription with a `TK` version prefix and a checksum that catches typos
  (`TK-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX-CC`). New CLI commands:
  `toku account secret-key generate` and `toku account emergency-kit`, the latter rendering
  the kit as plain text, self-contained printable HTML, or PDF. The Secret Key is surfaced
  once and is never sent to the server. Recovery semantics — including that lost secrets with
  no local copy mean server data is unrecoverable, and that local SQLite is the ultimate
  recovery — are documented in `docs/recovery.md`

- SRP-6a authentication for sync libraries: the sync server never sees the user's
  password or Secret Key. Libraries protected with a passphrase use SRP (RFC 5054,
  Group 2048 + SHA-256) so the server stores only a verifier (`v = g^x mod N`).
  First device enrolls via `POST /api/v1/auth/enroll`; any subsequent device logs in
  with `POST /api/v1/auth/challenge` + `POST /api/v1/auth/verify` and then calls
  `POST /api/v1/register` with the resulting session token. Session tokens are
  256-bit random values with a 24-hour TTL, SHA-256 hashed at rest
- Account rate-limiting for SRP libraries: 5 consecutive failed login attempts lock
  the account for 15 minutes (HTTP 423); successful login resets the counter

- User accounts, admin roles, and multi-user schema in the sync server
  (Immich-style self-hosting). A new `users` model stores SRP credentials and
  wrapped key material; accounts sign up via `POST /api/v1/account/signup` and log
  in with `POST /api/v1/account/challenge` + `POST /api/v1/account/verify`
  (user-scoped session tokens). The **first account** on a fresh instance becomes
  the administrator; subsequent self-registration is **closed by default** — the
  instance is invite/admin-gated. Admin endpoints let an administrator list users
  (`GET /api/v1/admin/users`), enable/disable accounts
  (`POST /api/v1/admin/users/{id}/status`, with guards against disabling yourself or
  the last admin), and toggle open registration (`GET`/`PUT /api/v1/admin/registration`).
  Libraries and devices gain a nullable owner (`user_id`), stamped when an
  authenticated user enrolls; legacy unowned rows remain valid. Honors "no social
  features": the only multi-user surface is administration, never cross-user data
  access

- Zero-knowledge multi-device key recovery in the sync server: signup now
  persists the account's wrapped **library data key** (`wrapped_data_key`,
  migration V7), and a new authenticated `GET /api/v1/account/keys` returns the
  account key bundle (`kdf_params`, `account_public_key`, `wrapped_private_key`,
  `wrapped_data_key`) a new device needs to unlock the shared library data key
  with its Secret Key + password. The server only ever stores and returns
  ciphertext plus the public key — it can never derive or read the data key. An
  account with no provisioned bundle returns `409 Conflict` rather than partial
  results. This unblocks new-device enrollment recovering the *same* data key as
  the original device, the zero-knowledge way

- Multi-device sync integration test harness (`toku-sync/tests/`): a reusable
  `TestServer` (real in-process Axum relay on a random port) and `SimulatedDevice`
  (real client + database + merge engine + deterministic HLC), plus 10 end-to-end
  scenarios covering basic sync, concurrent edits (same and different fields),
  delete propagation, delete-vs-edit, offline/reconnect, new-device bootstrap,
  encryption round-trip, idempotent push, and network-failure recovery. The
  `toku-sync` server router is now exposed as a library (`build_router`) so it can
  be driven in-process, and the token store honors `TOKU_TOKEN_STORE=file` /
  `TOKU_DISABLE_KEYCHAIN` to skip the OS keychain in CI
- Native conflict resolution (FFI + Apple): new `toku_sync_conflicts`, `toku_sync_resolve_conflict`, and `toku_sync_resolve_all_conflicts` C FFI functions surface the sync conflict log to Swift. The iOS and macOS apps gain a shared `ConflictResolutionView` (in `TokuKitUI`) listing unresolved note/review conflicts with per-conflict "Keep Local"/"Keep Remote" and bulk resolve actions, reachable from the sync settings screen
- Sync status indicator in the Apple apps: a shared `SyncStatusBadge` shows current sync state and surfaces pending conflicts — in the macOS sidebar, the iPad sidebar, and as a badge on the iOS "More" tab — routing to conflict resolution when conflicts need review
- Background sync push in the Apple apps: when sync is configured, the apps run a best-effort `push` when moving to the background/inactive (complementing the existing sync-on-launch pull), so local changes propagate without manual action
- Web sync status page (`/sync`): a read-only overview of the local sync configuration — server, device name/id, library id, encryption on/off, pending ops, push/pull cursors — with a link to the conflicts page and a "sync not configured" empty state; the dashboard header sync badge now links here
- Desktop (Windows) sync conflict notifications: the Tauri tray app polls the local database for unresolved conflicts and raises a system notification when new conflicts appear, keeps the tray tooltip updated with the current count, and adds a "Resolve conflicts…" tray menu item that opens the conflicts page
- Docker deployment for the self-hosted `toku-sync` server: a multi-stage, multi-arch
  (`linux/amd64` + `linux/arm64`, incl. Raspberry Pi) image published to
  `ghcr.io/kafkade/toku-sync`, plus a root `docker-compose.yml`, a built-in container health check
  (`toku-sync healthcheck`), and a `docs/sync-server.md` deployment guide covering Docker Compose
  and Caddy/nginx HTTPS reverse-proxy setup
- Structured logging for `toku-sync` via `tracing`/`tracing-subscriber` with per-request tracing;
  configurable through `--log-level` / `TOKU_SYNC_LOG_LEVEL` (and `RUST_LOG`)
- Sync conflict resolution UI (CLI + web): `toku sync conflicts` lists unresolved note/review conflicts, `toku sync conflicts show <id>` shows the local/remote diff, and `resolve <id> --keep local|remote` / `resolve-all --keep local|remote` resolve them. Resolving writes the chosen value to the entity, marks the conflict resolved, and emits a propagating sync op. `toku sync status` now reports the unresolved conflict count
- Web conflicts page (`/conflicts`): side-by-side local/remote diff cards with one-click keep-local / keep-remote buttons and bulk resolve actions, plus a sync status indicator in the dashboard header (shown only when sync is configured) that links to the conflicts page and highlights when conflicts are pending
- Native client sync (FFI + Swift): the iOS and macOS apps can now configure sync, push, pull, and view sync status/devices directly. New `toku_sync_init/push/pull/status/devices` C FFI functions expose the sync client to Swift, backed by a shared `toku-sync-client` crate extracted from the CLI so both surfaces reuse one implementation
- `SyncSettingsView` in the Apple apps (shared via `TokuKitUI`): set up sync against a server, enable optional end-to-end encryption, run "Sync Now"/push/pull, and view pending changes and registered devices — reachable from macOS Settings and the iOS Sync screen
- Automatic sync-on-launch in the Apple apps: when sync is configured, the iOS and macOS apps run a best-effort push/pull once on launch and refresh the library and stats if remote changes were applied; failures are silent (non-blocking)
- Snapshot compaction: `toku sync compact` creates a point-in-time snapshot of the entire library and uploads it to the server, pruning old ops to prevent unbounded op-log growth
- New-device bootstrap via snapshot: `GET /api/v1/snapshot` downloads the latest snapshot, `POST /api/v1/snapshot` uploads one with automatic op pruning
- `toku sync init` command for one-step sync setup: registers device, stores credentials, saves sync config to `config.toml`, with optional `--passphrase` for enabling encryption
- `toku sync status` command showing sync state: server, device, library, pending ops, cursors, encryption status, and registered device count
- `toku sync disable` command to remove sync configuration while preserving local data
- Sync config persisted in `config.toml` under `[sync]` section — push, pull, devices, rekey, and compact no longer require `--server` on every invocation
- Automatic device naming from hostname when `--device-name` is not specified during `toku sync init`
- Notes and reviews merge: LWW with conflict detection — two devices editing the same note stores a `sync_conflict` for user review, while edits to different notes merge cleanly
- Review field-level merge: content and rating are tracked independently, so two devices editing different review fields produces no conflict
- Settings sync: LWW per key for user settings with HLC-based ordering
- Notes, reviews, and user settings tables (migration V15) with soft delete support for notes and reviews
- Client-side encryption for sync ops: AES-256-GCM with Argon2id key derivation (m=64MB, t=3, p=1)
- Encrypted ops envelope per ADR-008: fields JSON is encrypted before leaving the device, server stores opaque blobs
- AAD (Additional Authenticated Data) binds envelope version, entity type, entity ID, and op type to prevent payload swapping between ops
- `SyncKey` with zeroize-on-drop for key material protection
- `SyncOp.encrypt()` and `SyncOp.decrypt()` convenience methods for in-place encryption/decryption
- `toku sync rekey --server <url>` command to change the sync encryption passphrase and re-encrypt all server-side ops
- Rekey server endpoint (`POST /api/v1/rekey`): atomically replaces all ops with re-encrypted versions, updates library salt, invalidates all device cursors
- Push lock during re-keying: server rejects push requests while a rekey is in progress to prevent corruption
- Salt endpoint (`GET /api/v1/salt`) for clients to fetch the library's current encryption salt
- Full-library pull endpoint (`GET /api/v1/pull/all`) for re-keying that includes the requesting device's own ops
- Sync key storage in OS keychain alongside auth tokens
- Sync data model: `sync_ops`, `sync_cursors`, and `sync_device` tables (migration V12) for local op-log staging
- Hybrid Logical Clock (HLC) implementation with fixed-width canonical format for causal ordering across devices
- `SyncRepository` for sync persistence: insert ops, query unpushed, mark pushed, device identity, cursor management
- `SyncOp` domain model with SHA-256 content checksums and `serde_json::Value` fields for canonical JSON
- Sync relay server (`toku-sync`): standalone Axum HTTP service that stores and relays sync operations between devices
- Device registration and authentication: `POST /api/v1/register` generates a 256-bit base64url auth token, stored as SHA-256 hash on the server
- Sync REST API: push ops, pull ops (with cursor-based pagination), device management, and health check
- `toku sync register` CLI command to register a device with a sync server and store the auth token in the OS keychain
- `toku sync devices` to list registered devices with `--format table|json|csv` support
- `toku sync deregister` to remove another device from the sync server
- `toku sync logout` to remove locally stored sync credentials
- Platform-native credential storage via OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) with secure file fallback
- `toku sync push --server <url>` to push local changes to the sync server with batched uploads and cursor tracking
- `toku sync pull --server <url>` to pull remote changes with automatic pagination and cursor-based resumption
- Pull endpoint excludes the requesting device's own ops to avoid echoing changes back
- `has_more` pagination flag on pull responses for batched retrieval of large op sets
- Entity-specific merge engine for applying remote sync ops to local state with per-entity strategies
- Book field-level last-write-wins (LWW) merge using HLC timestamps — two devices editing different fields produces no conflict
- Reading session append-only merge: remote sessions are inserted if new, never updated or deleted via sync
- Reading progress monotonic merge: remote progress values are only accepted if they exceed the current local maximum
- Tag sync: add/remove operations applied directly with deduplication
- Soft delete for books via `deleted_at` column, driven by sync delete ops with HLC timestamps
- Reading status transition validation during sync merge (rejects illegal state machine transitions)
- `sync_conflicts` table for future note/review conflict resolution UI
- Soft delete for books: `toku bulk delete` now sets `deleted_at` instead of removing the row, preserving data for sync propagation
- Deleted books are automatically excluded from all queries (list, search, stats, shelves, tags, FTS)
- `toku sync purge --days N` command to permanently remove tombstoned books after the retention period
- Delete operations create sync ops automatically when a device identity is registered, enabling cross-device delete propagation
- Web statistics dashboard (`toku serve`): reading stats, rating histogram, monthly pace chart, format breakdown donut, top authors and tags — all rendered as server-side SVG with dark mode support
- Yearly wrap-up pages at `/stats/wrap/{year}` summarizing a single year's reading
- JSON statistics API at `/api/stats` for programmatic access
- Web import wizard with step-by-step flow: upload CSV or enter Calibre path → dry-run preview → live progress via SSE → results summary
- Real-time import progress streaming with Server-Sent Events and automatic reconnect replay
- Support for Goodreads, StoryGraph, and Calibre imports through the web interface
- Library grid view with book covers, titles, authors, ratings, and status badges in a responsive layout
- Library list view with sortable table (title, author, status, rating, format, pages, date added)
- Book detail page with full metadata, reading sessions timeline, progress log, and tags grouped by type
- FTS5-powered search with status and tag filter dropdowns
- Cover image serving with content-addressed caching
- Pagination for large libraries (60 books per page)
- Filter bar with status, tag, sort controls, and grid/list view toggle
- ADR-007: Web framework decision documenting the choice of Axum + maud over HTMX and Leptos
- Windows desktop application (`toku-desktop`) wrapping the web UI in a native Tauri v2 window with system tray and minimize-to-tray support
- Extended FFI API with 9 new functions: delete, update status/rating, search, stats, tags, shelves, and Goodreads import
- macOS app (`toku-apple`) with SwiftUI: sidebar navigation, sortable table and grid views, book detail inspector, Swift Charts statistics dashboard, and drag-and-drop Goodreads CSV import
- iOS app (iPhone + iPad) with SwiftUI: library cover grid, book detail, barcode scanner (ISBN-13 via camera), quick progress update sheet, statistics glance with Swift Charts, full-text search, Goodreads CSV import, adaptive navigation (TabView on iPhone, NavigationSplitView on iPad), status filter chips, and pull-to-refresh
- TokuKitUI shared SwiftUI component library (StarRatingView, FlowLayout, MetadataRow, StatCard) for reuse across macOS and iOS apps
- 1Password-style account key hierarchy primitives in `toku-core` for hosted sync: two-secret unlock key derivation (password + Secret Key), wrapped account private keys, wrapped library data keys, and versioned serialized key-material formats for future migrations
- Authenticated, Secret-Key-gated device enrollment for the sync server: new devices
  are enrolled through an authenticated account session (which already proves
  possession of the password + Secret Key via SRP) at `POST /api/v1/devices/enroll`,
  rather than via open registration. Devices and libraries are scoped to the
  authenticated account, so a user can only enroll into libraries they own. Account
  holders can list and remove their own devices with
  `GET /api/v1/account/devices` and `DELETE /api/v1/account/devices/{id}`
- Optional trusted-device approval flow for sync (off by default, Immich-style
  opt-in): when enabled, a newly enrolled device on a library that already has an
  active device is held in a `pending` state and issued no session token until an
  existing trusted device approves it via `POST /api/v1/devices/{id}/approval`;
  the approved device then claims its token from `POST /api/v1/devices/{id}/session`.
  Rejected devices are denied. Administrators toggle the requirement with
  `GET`/`PUT /api/v1/admin/device-approvals`
- First-run onboarding and session authentication for the web dashboard. `toku serve`
  now has two modes: the default **local** mode is unchanged (no login, binds loopback
  only, and refuses non-loopback hosts), while `toku serve --hosted` requires sign-in so
  the dashboard can be exposed on a network. On first run, hosted mode walks you through
  creating an admin account and shows your **Emergency Kit** (email + Secret Key) once.
  Sign-in verifies your password server-side (SRP verifier, constant-time compare), issues
  a fresh 24-hour session cookie, and locks the account for 15 minutes after 5 failed
  attempts. All forms are CSRF-protected and `/healthz` stays public for liveness probes.
  The trusted-server trade-off (the hosted dashboard renders your decrypted library
  server-side) is documented in `docs/web-auth.md`

### Changed

- **Client-side E2E encryption is now mandatory for hosted/sync mode (zero-knowledge).**
  `toku sync init` always prompts for an encryption passphrase — the passwordless,
  plaintext opt-out has been removed. Every op payload and every snapshot is encrypted
  on-device before upload; the server stores only ciphertext and rejects plaintext
  uploads with HTTP 422. Snapshots, previously stored server-side as plaintext, are now
  encrypted too. Local-only single-device usage is unaffected (it never uploads). See
  `docs/sync-server.md` and ADR-010 for the zero-knowledge guarantee and the documented
  server-visible metadata (op/device ids, HLC, entity/op type — never content)
- Sync device registration is now gated once an instance has accounts: the legacy
  unauthenticated `POST /api/v1/register` path is rejected with 403 as soon as the
  first user account exists, so account-managed instances enroll devices only
  through the authenticated flow. Fresh, accountless instances (library-SRP relays)
  are unaffected
- Sync session validation now requires an `active` device: a session token belonging
  to a `pending` or `rejected` device is rejected even if the session row still exists
- `toku sync register` replaced by `toku sync init` with automatic defaults (hostname-based device name, auto-generated library ID)
- `toku sync push`, `pull`, `devices`, `rekey`, and `compact` no longer require `--server` flag — server URL is read from sync config
- `toku sync logout` replaced by `toku sync disable` which also clears sync config
- iOS and iPad app now displays as `toku` (lowercase) on the home screen

### Deprecated

- `toku sync init` and its `--passphrase` flag are deprecated in favor of
  `toku sync signup` (new account) and `toku sync login` / `toku sync enroll`
  (account + Secret Key auth). `init` still works and prints a notice pointing to
  the account commands

### Removed

- `toku sync register` command (replaced by `toku sync init`)
- `toku sync logout` command (replaced by `toku sync disable`)
- `--server` flag from push, pull, devices, rekey, and compact subcommands (now read from config)

### Fixed

- Sync pull now materializes pulled ops into local state. Previously the client
  staged remote ops but never applied them through the merge engine, so pulled
  changes never reached the `books`/library tables
- Sync now works end-to-end with client-side encryption across multiple devices:
  encrypted op payloads are carried over the wire (push encrypts, pull decrypts),
  and the key-derivation salt is coordinated through the server at `init` (first
  device establishes it, later devices adopt it) so every device derives the same
  key. Previously each device derived a key from its own random salt and could not
  decrypt peers' ops
- Sync init now adopts the server-assigned device id as the local device identity.
  Previously the local op-emitting device id differed from the id the server used
  to exclude a device's own ops on pull, so a device could pull back its own ops
- iOS and iPad app now builds and launches (previously authored but never compiled): fixed a missing `TokuKit` import, replaced the iOS 18-only tab API with an iOS 17 compatible tab bar, corrected an FFI status-code type mismatch, and added the `Info.plist` keys (`CFBundleExecutable`, `CFBundlePackageType`, etc.) required for installation

## [0.2.1] - 2026-05-30

### Changed

- Update versioning mechanism.

## [0.2.0] - 2026-05-30

### Added

- Work grouping: `toku work link|unlink|show|auto` to group multiple editions of the same creative work, with automatic candidate detection by normalized title and primary author
- Duplicate merging: `toku merge <keep> <remove>` moves all reading sessions, progress, tags, authors, ISBNs, and metadata from the removed book to the kept book in a single transaction
- StoryGraph CSV import: `toku import storygraph <file>` with mood tags, pace ratings, content warnings, quarter-star rating conversion, multi-session date parsing, contributor roles (narrator/translator), and DNF/paused status mapping
- Smart shelves: `toku shelf create "Unread Sci-Fi" --smart --filter "status:want_to_read AND tag:sci-fi"` creates saved filter rules that auto-populate with matching books
- Filter DSL for smart shelves supporting 11 fields (status, tag, mood, pace, rating, pages, author, format, shelf, pub_date, date_added) with comparison operators and AND/OR/parentheses
- `toku shelf list|show|delete|add|remove` for managing both regular and smart shelves with `--format table|json|csv` output
- Interactive TUI library browser (`toku browse`, or just `toku` with no subcommand) with split-pane layout: scrollable book list on the left, live detail view on the right
- Tag display in TUI book detail pane with styled cyan labels
- Filter popup in TUI browser to narrow books by reading status or tag
- Ratatui-based import progress UI with live progress bar, per-row activity log, and colored status indicators
- Structured import summary with status breakdown, sample lists of imported/skipped/updated books, and undo instructions
- Width-aware `toku list` table output that adapts columns to terminal width with modern rounded borders
- Open Library search: `toku search --online <query>` searches the Open Library API and displays results with ISBNs for easy adding
- Bulk operations: `toku bulk tag|status|delete` for applying tags, changing status, or deleting multiple books at once, with `--dry-run` and filter flags (`--status`, `--tag`)
- `toku add --tag <tag>` (`-T`) flag to apply tags when adding a book
- `toku add --status <status>` flag to set reading status when adding a book (creates a reading session automatically for `--status reading`)
- Goodreads import now converts the `Bookshelves` CSV column into tags, preserving user shelf organization
- Goodreads re-import updates tags on existing books instead of silently skipping them (shown as `Updated` in progress UI)
- Non-standard Goodreads exclusive shelves (e.g., custom shelves like "favorites") are preserved as tags to avoid data loss

### Changed

- Shared import types (ImportReport, ImportEvent, ImportObserver) extracted to common module for reuse across all importers
- Goodreads importer now uses observer pattern for progress reporting and wraps non-dry-run imports in a transaction for atomicity
- Import report includes bounded sample lists (up to 20) of imported, updated, and skipped books with status counts
- Shelves merged into tags — all user-created groupings are now tags; `ReadingStatus` remains as the separate state machine for tracking reading progress
- Removed `toku shelf` command — use `toku tag` instead (existing shelf data migrated to tags via DB migration V8)
- Removed `--shelf` filter from `toku list` and `toku search` — use `--tag` instead

### Fixed

- Multibyte string truncation panic when book titles or descriptions contain non-ASCII characters
- `toku list` no longer dumps the entire library after import — replaced with structured summary
- Project README with "Why Toku?" naming rationale and CLI usage examples
- Product roadmap covering 9 phases from MVP to moonshots
- Architecture Decision Records: core language (ADR-001), database schema (ADR-002), CLI design (ADR-003), metadata sources (ADR-004), import architecture (ADR-005), sync strategy (ADR-006)
- CI workflow: markdown linting + Rust build/test/clippy on Linux, macOS, and Windows
- Release workflow: cross-platform binary builds + crates.io publishing
- Release script for automated version bumping, changelog stamping, and tagging
- Contributing guide with importer contribution instructions
- GitHub issue templates for bug reports and feature requests
- PR template with data integrity checklist
- Cargo workspace with `toku-core`, `toku-db`, and `toku-cli` crates
- Book domain model: books, authors (with roles), series, reading status, and book format types
- ISBN-10 and ISBN-13 validation with check digit verification and bidirectional conversion
- SQLite database with FTS5 full-text search, auto-synced via triggers
- Book persistence: create, list, search, delete books; manage authors and ISBNs
- `toku --version` CLI entry point
- Open Library metadata fetching: `toku add --isbn <isbn>` fetches title, author, pages, language, and cover image
- Cover image downloading with content-addressed local storage (SHA-256)
- `toku add --title <title> --author <author>` for manual book entry
- `toku show <book>` with full detail view (title, author, status, pages, cover, description)
- `toku list` with formatted table output, filterable by `--status`
- `toku search <query>` with FTS5 full-text search
- `--format table|json|csv` output modes for all list/search commands
- Goodreads CSV import: `toku import goodreads <file>` with dry-run, idempotent re-import, ISBN cleaning, rating conversion, status mapping, and format detection
- Import rollback: `toku import undo <import-id>` removes all books from a specific import
- Import provenance tracking per field for future re-import safety
- Reading status management: `toku reading start|finish|abandon|hold|resume` with state machine validation and automatic date tracking
- Reading sessions with per-session ratings and notes
- Shelves: `toku shelf create|add|remove|list` for user-defined book collections
- Tags: `toku tag add|remove|list` with case-insensitive matching
- `toku list --shelf <name>` and `toku list --tag <name>` filters
- Full-text search now includes author names alongside title and description
- `toku search` with `--status`, `--shelf`, and `--tag` filters for narrowing results
- Configuration file (`config.toml`): default output format, color mode, metadata source
- `toku config` to view settings, `toku config --edit` to open in editor
- `toku completions bash|zsh|fish|powershell` for shell completion generation
- Reading progress tracking: `toku reading update --page|--percent|--chapter|--duration` with timestamped log entries
- `toku reading log <book>` to view reading progress history
- Duration parsing for audiobooks (`5h30m`, `330m`, `5.5h`)
- Calibre library import: `toku import calibre <path>` with books, authors, series, tags, covers, and ISBNs
- Calibre import supports `--dry-run` and `--no-covers` flags
- Calibre HTML descriptions automatically stripped to plain text
- Reading statistics: `toku stats` with books/pages read, average rating, reading pace, and format breakdown
- `toku stats --year 2025` for year-filtered analytics
- Currently reading list with progress percentages in stats output
- Full statistics engine: rating distribution, reading streaks, monthly books finished, shortest/longest book, average days to finish, reading speed (pages/hour), top authors, and top tags
- `toku stats --author <name>` to filter all statistics to a single author's books
- Mood tags, pace ratings, and content warnings as typed tag categories (`toku edit --mood adventurous --pace fast --content-warning violence`)
- `toku edit` command for updating mood tags, pace, content warnings, and rating on existing books (with `--remove-mood` and `--remove-content-warning` for removal)
- `toku add --mood`, `--pace`, and `--content-warning` flags for setting typed tags at add time
- `toku list --mood <tag>` and `--pace <rate>` filters with same-type OR, cross-type AND semantics
- `toku stats --mood-trends` showing mood tag distribution per month across finished books
- `toku show` now displays mood tags, pace rating, and content warnings in detail view (all output formats)
- `toku tag list` now shows tag type column (general, mood, pace, content_warning)
- Export to CSV: `toku export csv` with flat book table (title, authors, status, rating, shelves, tags)
- Export to JSON: `toku export json` with full structured library data
- Export to Markdown: `toku export markdown` with books grouped by reading status and star ratings
- Canonical backup: `toku export backup --output toku-backup.zip` with library data + cover images in a self-contained ZIP

[Unreleased]: https://github.com/kafkade/toku/compare/v0.2.1...HEAD
