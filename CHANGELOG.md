# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/kafkade/toku/compare/v0.1.0...HEAD
