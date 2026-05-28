# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Interactive TUI library browser (`toku browse`, or just `toku` with no subcommand) with split-pane layout: scrollable book list on the left, live detail view on the right
- Filter popup in TUI browser to narrow books by reading status, shelf, or tag
- Ratatui-based import progress UI with live progress bar, per-row activity log, and colored status indicators
- Structured import summary with status breakdown, sample lists of imported/skipped books, and undo instructions
- Width-aware `toku list` table output that adapts columns to terminal width with modern rounded borders

### Changed

- Goodreads importer now uses observer pattern for progress reporting and wraps non-dry-run imports in a transaction for atomicity
- Import report includes bounded sample lists (up to 20) of imported and skipped books with status counts

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
- Export to CSV: `toku export csv` with flat book table (title, authors, status, rating, shelves, tags)
- Export to JSON: `toku export json` with full structured library data
- Export to Markdown: `toku export markdown` with books grouped by reading status and star ratings
- Canonical backup: `toku export backup --output toku-backup.zip` with library data + cover images in a self-contained ZIP

[Unreleased]: https://github.com/kafkade/toku/compare/v0.1.0...HEAD
