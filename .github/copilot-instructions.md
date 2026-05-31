# Copilot Instructions for Toku

## Project Overview

Toku (読く — Japanese for "to read") is a private, offline-first personal book manager. It combines the metadata depth of Calibre, the reading tracking of Goodreads, and the analytics of StoryGraph — without any social features. Phases 0–5 are complete (CLI, imports, analytics, web dashboard, native apps). Phase 6 (file management) and Phase 7 (sync) are next.

## Non-Negotiable Constraints

Every code contribution, architecture decision, and feature design must uphold these:

1. **Local-first** — The app must work fully offline. All core features (library management, reading tracking, statistics, search, import, export) function without an internet connection. Network access is only for optional metadata enrichment.
2. **No social features** — No friends, followers, feeds, book clubs, or shared reviews. This is a personal tool. The user is the sole audience for their data.
3. **User data ownership** — The user owns 100% of their data in portable, open formats. Full export at any time. No vendor lock-in.
4. **Import as a superpower** — Importing from Goodreads, Calibre, and StoryGraph must be frictionless, idempotent, and lossless. Import quality is a first-impression feature.
5. **CLI-first** — The CLI is the primary interface and a first-class product. Web, iOS, macOS, and Windows are future platforms built on the same core library.

## Architecture

Cargo workspace with 9 crates:

- `toku-core/` — Domain models, traits, state machine, statistics engine. Pure Rust, no I/O. Compiles to native, WASM, FFI.
- `toku-db/` — SQLite persistence, schema migrations (refinery), FTS5 full-text search.
- `toku-import/` — Import implementations: Goodreads CSV, Calibre metadata.db, StoryGraph.
- `toku-meta/` — Metadata fetching: Open Library API (primary), Google Books (fallback). Cover image downloading.
- `toku-cli/` — CLI binary (clap v4). The main entry point.
- `toku-export/` — Export implementations: CSV, JSON, Markdown, BibTeX, canonical backup.
- `toku-ffi/` — C FFI bindings for Swift/Kotlin via `cbindgen`. Used by macOS and iOS apps.
- `toku-web/` — Axum + maud web server. Library views, statistics dashboard, import wizard. Started via `toku serve`.
- `toku-desktop/` — Tauri v2 Windows desktop app wrapping the web UI.

### Data Boundary Rule

| Data Type | Storage | Network? |
|---|---|---|
| User's book library, reading sessions, notes, ratings | Local SQLite (encrypted at rest if sync enabled) | Never sent unless sync opted in |
| Book metadata from APIs | Cached locally after fetch | Open Library / Google Books (optional) |
| Cover images | Local filesystem, content-addressed (SHA-256) | Fetched once, stored locally forever |
| Import source data (Goodreads CSV, Calibre DB) | Parsed on-device, stored in local DB | Never leaves device |
| Statistics and analytics | Computed locally from user data | Never sent anywhere |

### Key Data Model Decisions

- **Book = Edition** for MVP. A nullable `work_id` column enables Work grouping in Phase 3.
- **Ratings**: 0–10 integer, displayed as 5★ with half-star increments. Goodreads-compatible.
- **Identifiers**: ISBN-10, ISBN-13, ASIN, Open Library ID, Goodreads ID. Books without ISBNs are fully supported.
- **Contributors**: Author, Editor, Translator, Illustrator, Narrator (audiobooks) — via BookAuthor join table with role enum.
- **Provenance**: Every metadata field tracks its source (user entry, Goodreads import, Open Library API) and timestamp. User edits always take precedence.

## Conventions

- **License**: MIT — all contributions must be compatible
- **PR title format**: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`
- **Commit trailer**: Include `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>` when Copilot contributes
- **Error handling**: `thiserror` for library crates, `anyhow` for CLI binary. No `unwrap()` in user-facing paths.
- **CLI output**: Respect `NO_COLOR` env var. Support `--format table|json|csv` for all list commands.
- **Database migrations**: Use `refinery` with embedded SQL migrations. Migrations are idempotent.
- **Import idempotency**: Every importer stores source IDs (e.g., `goodreads_id`) for dedup on re-import. User edits are never overwritten.

## Git Policy

**Never execute Git commands that modify history or submit code.** This includes `git commit`, `git push`, `git rebase`, `git merge`, `git reset`, `git cherry-pick`, `git revert`, and `git tag`. Read-only commands like `git status`, `git diff`, `git log`, and `git branch` are fine. The maintainer must always review and commit changes themselves.

## CI / Infrastructure Dependency

**Branch protection for this repo is managed via Terraform in `kafkade/github-infra` (`repo_toku.tf`).** The `required_status_checks` list must match the job names in `.github/workflows/ci.yml`. If you rename, add, or remove CI jobs that are used as merge gates (currently `Validate`), the corresponding IaC config must be updated or PRs will be permanently blocked. Always flag this when proposing workflow changes.

## Reference Documents

- Full product roadmap: `ROADMAP.md`
- Architecture Decision Records: `docs/adr/`
- CLI command reference: see ADR-003 (`docs/adr/003-cli-design.md`)
