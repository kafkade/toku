# 📚 Toku — Product Roadmap

> **A private, offline-first, multi-platform personal book manager.**
> CLI-first. Your data, your rules. No social features. No cloud dependency.

**Repository**: `kafkade/toku`
**Core Language**: Rust
**Primary Interface**: CLI
**Date**: May 2026
**Author**: kafkade

---

## Section 0: Assumptions Table

| # | Question | Default | Reasoning | Risk if Wrong |
|---|----------|---------|-----------|---------------|
| 1 | Team size | Solo developer + community contributions | User-confirmed. Phases limited to 3–5 deliverables. | Overscoped phases stall development |
| 2 | Timeline | Open-ended, ship when ready | Solo developer — milestone-driven deadlines create burnout. Quality > speed. | No external pressure to ship, risk of infinite polishing |
| 3 | Budget | $0 — no paid APIs, no paid infra | Open source project. Use only free-tier APIs (Open Library, Google Books free quota). | Must fall back to manual entry if free APIs degrade |
| 4 | MVP focus | CLI only | Web UI adds weeks of work and a JS build chain. CLI validates the data model and core library first. | Audience limited to terminal users until Phase 4 |
| 5 | Distribution | `cargo install` + pre-built binaries (GitHub Releases) | Cargo reaches the Rust community. Pre-built binaries reach everyone else. Homebrew tap in Phase 2. | Users without Rust toolchain need binaries — CI must cross-compile |
| 6 | File management | Defer to post-1.0 (Phase 6) | File management (ebook storage, conversion, OPDS) is a separate product concern. MVP is a reading tracker, not a file manager. | Power Calibre users may wait for file features before switching |
| 7 | Primary persona | Privacy-Conscious Power Reader | Confirmed: 100–500 books, Goodreads user, values data ownership. CLI-comfortable. | If casual readers are the real audience, CLI-first is wrong |
| 8 | Wedge persona | Goodreads refugee who lives in the terminal | Switching trigger: "Amazon owns my reading data and Goodreads is rotting" | If Goodreads improves, switching motivation weakens |
| 9 | Book formats tracked | Physical books, Ebooks (Kindle/Kobo), Audiobooks | User-confirmed. Data model must handle page-based and time-based progress from day one. | Audiobook support adds complexity to progress tracking |
| 10 | Import priority | Goodreads CSV first (user's current tool) | User-confirmed. This is the first-impression feature. | If Goodreads changes CSV format, MVP import breaks |
| 11 | Primary metadata source | Open Library (primary) + Google Books (fallback) | Open Library: free, no key required, community-maintained. Google Books: broader coverage, 1,000 req/day free. | Open Library has gaps for non-English and niche titles |
| 12 | Edition model | Start with Book = Edition, add Work grouping in Phase 3 | Full FRBR is overengineered for MVP. 100–500 books rarely has edition conflicts. | Migration from flat to hierarchical requires careful schema design |
| 13 | Core language | Rust | User-confirmed. Excellent CLI ecosystem (clap), cross-compilation, WASM target, strong community. | Higher contribution barrier than Go/Python |
| 14 | Database | SQLite with FTS5 | Battle-tested embedded DB. Single-file. Cross-platform. FTS5 for search. | No issues at 500 books. At 50k+ books, may need query optimization |
| 15 | Rating scale | 5-star with half-star increments (stored as 0–10 integer) | Matches Goodreads import (1–5 stars), allows more precision. Simpler than 10-point for casual use. | Half-stars complicate CLI input slightly |
| 16 | License | MIT | Maximum contributor friendliness, no copyleft friction, standard in Rust ecosystem. | No copyleft protection — forks can close source |
| 17 | GitHub org | kafkade | User-confirmed. Repo: `kafkade/toku`. | Name must work as `kafkade/<name>` on GitHub |
| 18 | Project name | Toku (recommended — see Section 17) | Short, punchy, non-English, terminal-friendly. Japanese 読く (to read). | May have pronunciation ambiguity outside East Asian contexts |

---

## Section 1: User Personas

### 1. Mara — The Privacy-Conscious Power Reader ⭐ Primary Persona

- **Archetype**: 30s, reads 40–60 books/year across physical and Kindle. Software engineer.
- **Library**: 300 books tracked in Goodreads, wants to leave Amazon's ecosystem.
- **Technical comfort**: High — lives in the terminal, uses Neovim, loves CLI tools.
- **Platforms**: macOS terminal (primary), iPhone (future — quick progress updates).
- **Pain point**: "Goodreads is owned by Amazon, the UI is stuck in 2012, and I can't export my reading history cleanly. My data isn't mine."
- **Switching trigger**: `toku import goodreads ~/export.csv` → full library in seconds → never opens Goodreads again.
- **Phase alignment**: MVP (Phase 1) — this is the launch user.

### 2. Dex — The Calibre Power User

- **Archetype**: 40s, 1,200+ ebooks managed in Calibre. Reads 30 books/year.
- **Library**: Massive ebook collection, meticulously tagged, uses Calibre daily for metadata.
- **Technical comfort**: Very high — runs Calibre server, uses command-line tools.
- **Platforms**: Linux terminal, potentially web UI for browsing.
- **Pain point**: "Calibre manages my files but can't track reading progress. I use a spreadsheet alongside it. It's 2025 and I have two systems."
- **Switching trigger**: Calibre import + reading progress tracking in one tool.
- **Phase alignment**: Phase 2 (Calibre import) — needs file management features in Phase 6 for full switch.

### 3. Lena — The Casual Tracker

- **Archetype**: 20s, reads 15 books/year, currently uses Goodreads sporadically.
- **Library**: ~80 books. Doesn't care about metadata depth. Wants "what did I read this year?"
- **Technical comfort**: Low-to-medium — can install an app, won't use a terminal.
- **Platforms**: Web UI (Phase 4), iOS (Phase 5).
- **Pain point**: "Goodreads is too social. I just want a private list."
- **Switching trigger**: Web UI with simple "add book, mark as read" flow.
- **Phase alignment**: Phase 4 (Web) — not reachable via CLI.

### 4. Prof. Navarro — The Academic Researcher

- **Archetype**: 50s, tracks 200+ academic books and papers alongside recreational reading.
- **Library**: Needs custom fields (course, semester, citation key), BibTeX export.
- **Technical comfort**: Medium — uses Zotero, LaTeX, comfortable with structured tools.
- **Platforms**: macOS terminal, web UI.
- **Pain point**: "Zotero is great for papers but awkward for novels. Goodreads can't handle textbooks properly."
- **Switching trigger**: Custom fields + BibTeX export + standard book tracking in one tool.
- **Phase alignment**: Phase 3 (custom fields, BibTeX export) — niche but vocal community.

### 5. Kai — The Data Nerd / Quantified Reader

- **Archetype**: 20s, reads 80+ books/year, obsessed with reading analytics.
- **Library**: 400 books, currently on StoryGraph for mood tracking and stats.
- **Technical comfort**: High — developer, loves dashboards and data visualization.
- **Platforms**: CLI + web stats dashboard.
- **Pain point**: "StoryGraph's stats are good but I can't customize them or run my own queries. I want SQL access to my reading data."
- **Switching trigger**: SQLite database they own + rich stats engine + CLI scriptability.
- **Phase alignment**: Phase 3 (full analytics) — will tolerate minimal stats in Phase 2.

### 6. Sam — The Developer / Contributor

- **Archetype**: 30s, primarily interested in the architecture. Reads 20 books/year.
- **Library**: Small. Main interest is contributing to the codebase.
- **Technical comfort**: Very high — Rust developer, wants clean APIs and good docs.
- **Platforms**: Whatever the project uses.
- **Pain point**: "I want to build something cool in Rust. Most book apps are proprietary or poorly architected."
- **Switching trigger**: Clean crate architecture, good documentation, welcoming contribution model.
- **Phase alignment**: Phase 0+ — contributor from the start.

---

## Section 2: Cross-Platform Architecture & Core Library Design

### 2.1 — Layered Architecture

```text
┌─────────────────────────────────────────────────┐
│                  Frontend Adapters               │
│  ┌─────┐  ┌─────┐  ┌──────┐  ┌──────┐  ┌─────┐│
│  │ CLI │  │ Web │  │ iOS  │  │macOS │  │ Win ││
│  └──┬──┘  └──┬──┘  └──┬───┘  └──┬───┘  └──┬──┘│
├─────┴────────┴───────┴────────┴─────────┴──────┤
│            Metadata Enrichment Layer             │
│  Open Library API · Google Books · Cover fetch   │
│  (optional — requires network)                   │
├─────────────────────────────────────────────────┤
│                  Data Layer                       │
│  SQLite + FTS5 · Migrations · Import/Export       │
│  Cover image storage · Data validation            │
├─────────────────────────────────────────────────┤
│                Domain Core Layer                  │
│  Book model · Reading sessions · Shelves/Tags     │
│  State machine · Stats engine · Search/Filter     │
│  Import/Export traits · No I/O, no network        │
└─────────────────────────────────────────────────┘
```

**Domain core layer** — Pure Rust, no I/O dependencies. Compiles to native, WASM, and C FFI. Contains:

- Book/Author/Series models with validation
- Reading state machine (WantToRead → Reading → Read | Abandoned | OnHold)
- Statistics computation engine (all calculations are pure functions over data)
- Search and filter logic
- Import/export trait definitions (not implementations)

**Data layer** — SQLite via `rusqlite`. Compiles to native (WASM via `sql.js` in future).

- Schema management with `refinery` for migrations
- FTS5 full-text search index
- Import implementations (Goodreads CSV, Calibre DB)
- Cover image filesystem management

**Metadata enrichment layer** — Network-dependent, always optional.

- Open Library and Google Books API clients
- Cover image downloading
- Rate limiting + response caching
- Merge logic: fill empty fields only; never overwrite user edits

**Frontend adapters** — Each is a separate crate/binary:

- `toku-cli`: Primary interface (Phase 0+)
- `toku-web`: Axum server + HTMX or Leptos frontend (Phase 4)
- `toku-ffi`: C FFI bindings for Swift/Kotlin (Phase 5)

### 2.2 — Workspace Layout

**Recommendation**: Monorepo with Cargo workspace from day one.

The crate count is small enough (9 crates) that the overhead is minimal, and the boundaries enforce the layered architecture. A monolith-first approach risks coupling the CLI to the core in ways that are painful to untangle when the web UI arrives.

```sh
kafkade/toku/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── toku-core/          # Domain: models, traits, state machine, stats
│   ├── toku-db/            # SQLite, migrations, FTS5, queries
│   ├── toku-import/        # Goodreads CSV, Calibre, StoryGraph parsers
│   ├── toku-meta/          # Open Library, Google Books API clients
│   ├── toku-cli/           # CLI binary (clap-based)
│   ├── toku-export/        # CSV, JSON, Markdown, BibTeX exporters
│   ├── toku-ffi/           # C FFI bindings for Swift/Kotlin (cbindgen)
│   ├── toku-web/           # Axum + maud web server (library crate)
│   └── toku-desktop/       # Tauri v2 Windows desktop app
├── toku-apple/             # macOS + iOS SwiftUI apps (Xcode project)
│   └── TokuKit/            # Swift FFI wrapper + shared UI components
├── docs/                   # User and developer documentation
├── tests/                  # Integration tests, test fixtures
│   └── fixtures/           # Sample Goodreads CSVs, Calibre DBs
├── .github/
│   └── workflows/          # CI: test, lint, build, release
├── README.md
├── LICENSE                 # MIT
└── CONTRIBUTING.md
```

### 2.3 — Database Architecture

**Primary database**: SQLite 3 with FTS5 extension.

- **Schema philosophy**: Normalized core entities (books, authors, series) with join tables for relationships. Denormalized FTS5 virtual table for search (rebuilt on import, updated on edit). This balances query simplicity with search performance.
- **Edition handling**: MVP uses a single `books` table where each row is an edition. A `work_id` nullable column is reserved — initially NULL, populated when Work grouping is added in Phase 3.
- **Cover images**: Stored on the filesystem in a `covers/` subdirectory, referenced by content hash (`sha256.jpg`). Thumbnails generated on first display. Rationale: keeps the SQLite file small and portable; cover files are large and binary.
- **Database location**:
  - Linux: `$XDG_DATA_HOME/toku/` (default `~/.local/share/toku/`)
  - macOS: `~/Library/Application Support/toku/`
  - Windows: `%APPDATA%\toku\`
  - Overridable via `TOKU_DATA_DIR` env var or `--data-dir` flag
- **Migration strategy**: `refinery` crate for versioned SQL migrations embedded in the binary. Each migration is idempotent. Migration runs automatically on app start.
- **Backup**: The entire `toku/` directory (SQLite file + covers/) is the backup. `toku export backup` creates a self-contained archive (see Section 3.1.2).

### 2.4 — Platform Compilation Targets

| Platform | Target | UI Framework | Database | Phase |
|----------|--------|-------------|----------|-------|
| Linux CLI | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | Terminal (clap + tabled) | SQLite (rusqlite) | **MVP** |
| macOS CLI | `x86_64-apple-darwin`, `aarch64-apple-darwin` | Terminal | SQLite | **MVP** |
| Windows CLI | `x86_64-pc-windows-msvc` | Terminal | SQLite | **MVP** |
| Web | Server-rendered (Axum + templates) | HTMX or Leptos | SQLite (server-side) | Phase 4 |
| iOS | `aarch64-apple-ios` | SwiftUI via FFI | SQLite | Phase 5 |
| macOS App | Native | SwiftUI | SQLite | Phase 5 |
| Windows App | Native | Tauri v2 | SQLite | Phase 5 |

### 2.5 — Sync Strategy

Sync is opt-in. Toku must work fully offline on a single device forever. Sync adds
convenience — it never adds capability. A user who never enables sync loses nothing.

#### Phase 1 (MVP): No sync — manual archive

Single-device. The `toku/` data directory can be manually backed up or synced via
Syncthing, iCloud Drive, or Dropbox. `toku export backup` creates a self-contained
archive (see Section 3.1.2) that can be imported on another machine. Document this as
a supported workflow for power users who want multi-device today.

#### Phase 7: Toku Sync — Lightweight REST sync with optional encryption

**Architecture**: Changeset-based sync over a thin REST API.

```text
┌──────────┐    ┌──────────┐    ┌──────────┐
│ CLI      │    │ iOS App  │    │ Web App  │
│ (local   │    │ (local   │    │(connected│
│  SQLite) │    │  SQLite) │    │  client) │
└────┬─────┘    └────┬─────┘    └────┬─────┘
     │               │               │
     └───────┬───────┘               │
             │ push/pull ops          │ API calls
             ▼                        ▼
     ┌───────────────────────────────────┐
     │         Toku Sync Server          │
     │  (Axum · SQLite · Docker/fly.io)  │
     │                                   │
     │  Stores: encrypted changesets,    │
     │  sync cursors, device registry,   │
     │  snapshots                        │
     └───────────────────────────────────┘
```

**How it works**:

1. **Every mutation is an operation (op)**. When a native client changes a book's rating,
   adds a reading session, or creates a shelf, it writes to the local SQLite AND appends
   an op to a local `sync_ops` table with a unique op ID + hybrid logical clock (HLC)
   timestamp.

2. **Push**: Client sends new ops since its last push cursor to the server. Ops are
   optionally encrypted client-side before upload (see encryption below).

3. **Pull**: Client requests ops since its last pull cursor. Applies them to local
   SQLite using entity-specific merge rules.

4. **The server is a dumb relay** — it stores ops, manages cursors per device, and
   serves snapshots. It does not need to understand the data (especially when encrypted).

5. **Snapshots**: Periodically, the server (or a client) compacts the op log into a
   full snapshot. New devices pull the latest snapshot + ops since snapshot, avoiding
   unbounded log replay.

**Entity-specific merge rules** — not one-size-fits-all:

| Entity | Merge Strategy | Rationale |
|--------|---------------|-----------|
| Books (metadata) | Last-write-wins per field (HLC) | User edits one field at a time; field-level LWW avoids overwriting unrelated changes |
| Reading sessions | Append-only | Sessions are immutable facts ("I read pages 50–100 on April 15"). No conflicts possible. |
| Reading progress | Monotonic — highest page/latest timestamp wins | Progress only moves forward. If two devices report page 145 and page 200, page 200 wins. |
| Shelves / Tags | Last-write-wins per entity | Renames and deletes are rare; LWW is sufficient. |
| Notes / Reviews | Last-write-wins with conflict detection | If two devices edit the same note, keep the later edit but store the earlier as a conflict version the user can review. |
| Reading status | State machine constraint — valid transitions only | Enforce: WantToRead → Reading → Read/Abandoned. Invalid transitions rejected. |
| Deletes | Soft delete with tombstone + 30-day retention | Tombstones prevent deleted books from reappearing after sync. Purge after 30 days. |
| Cover images | Content-addressed (hash), lazy-fetch | Not in the critical sync path. Metadata syncs the hash; images download on demand. |
| Import logs | Never sync | Device-local only. |
| Settings | Last-write-wins per key | Settings changes are infrequent and intentional. |

**Web app model** — the web app is a **connected companion**, not a full local-first
client:

- Web v1 talks directly to the sync server's API — it reads/writes via REST, not local
  SQLite. This avoids the complexity of browser-based SQLite (OPFS/sql.js) for a solo
  developer.
- The server is the web app's "database." Native clients are authoritative; the web is
  a convenient window.
- Future: if browser-local SQLite matures (via OPFS + sqlite-wasm), the web app can
  adopt the same local-first model as native clients. This is not required for v1.

**Device management**:

- Each device registers with a device ID (UUID) and a human-readable name ("Javier's
  MacBook", "iPhone").
- Devices are listed in `toku sync devices` and can be deregistered.
- No concept of "revoking" a device's data — if a device is lost, the user changes
  their sync passphrase, which re-encrypts all server-side data with a new key.

#### Encryption and authentication — superseded by ADR-010

> **⚠️ This section reflects the original ADR-006 design (optional encryption,
> unauthenticated relay) and has been superseded.** See
> [ADR-010](adr/010-self-host-auth.md) for the current model: self-hosted server with
> mandatory zero-knowledge E2E encryption and 1Password-style two-secret SRP
> authentication.

~~Book data is personal but not health-critical. Toku does **not** need Pildora's
zero-knowledge architecture (SRP, vault hierarchy, per-item keys, asymmetric key
exchange). Instead:~~

**Current model (ADR-010)**: mandatory zero-knowledge E2E with 1Password-style SRP auth.

- **Two secrets**: account password + high-entropy Secret Key generated on the device
  at sign-up. Neither is ever sent to the server.
- **SRP authentication**: server stores only an SRP verifier; it cannot derive the
  password, Secret Key, or any encryption key.
- **Key hierarchy**: (Secret Key + password) → Argon2id → Account Unlock Key → wraps
  Account Key Pair → wraps Library Encryption Key. The server stores only encrypted blobs.
- **Mandatory encryption for hosted mode**: there is no plaintext fallback.
- **Emergency Kit**: Secret Key is surfaced once at registration. Losing it with no local
  device means server data is unrecoverable. Local SQLite is the recovery path.

**What the server can and cannot see** remains the same as before: op count, timestamps,
device IDs, entity types — but never content. ADR-010 makes this a hard guarantee rather
than a user-configurable option.

#### Deployment options

- **Self-hosted (recommended)**: Docker image + docker-compose, single binary, SQLite
  backend. First-run admin onboarding. Run on a VPS, NAS, or Raspberry Pi.
  `docker compose up kafkade/toku-sync`.
- **Managed (future, maybe)**: A hosted instance at `sync.toku.dev` for users who don't
  want to self-host. Free tier for small libraries. This is optional and deferred — the
  self-hosted path must work first.

Rejected sync alternatives:

- **cr-sqlite**: Elegant but experimental. iOS/WASM support is unproven. Kept as a research branch — if it matures, it could replace the op-log approach for native clients. Not the roadmap bet.
- **File-based sync (iCloud/Dropbox)**: SQLite files + cloud sync = corruption risk. Documented as a manual backup workflow, not a sync strategy.
- **Turso/libSQL**: Adds a mandatory cloud dependency. Conflicts with local-first.
- **Optional-only encryption (original ADR-006 model)**: Replaced by ADR-010 — unauthenticated relay is not viable for a self-hosted server facing the public internet.

#### Sync protocol — technical summary

```sh
POST /sync/push     — Client sends new ops (encrypted or plaintext)
GET  /sync/pull     — Client receives ops since cursor
GET  /sync/snapshot — Client downloads latest compacted snapshot
POST /sync/register — Register a new device
GET  /sync/devices  — List registered devices
DELETE /sync/device — Deregister a device
POST /sync/rekey    — Upload re-encrypted snapshot after passphrase change
```

Each op:

```json
{
  "op_id": "uuid-v7",
  "device_id": "uuid",
  "hlc": "2025-07-14T10:30:00.000Z-0001-deviceA",
  "entity_type": "book",
  "entity_id": "uuid",
  "op_type": "update",
  "payload": "<encrypted-or-plaintext JSON>",
  "checksum": "sha256"
}
```

---

## Section 3: Data Sources, Import & Export

### 3.1 — Import Source Matrix

| Source | Format | Key Data | Complexity | Phase | Validation |
|--------|--------|----------|-----------|-------|------------|
| **Goodreads** | CSV export | Title, author, ISBN, rating (1–5), date read, shelves, review, page count, Goodreads ID | 🟢 | **MVP** | `[Validation Required]` — verify current CSV column names |
| **Calibre** | SQLite `metadata.db` + OPF | Full metadata, tags, custom columns, series, publisher, covers, file paths | 🟡 | Phase 2 | `[Validated]` — well-documented, stable schema |
| **StoryGraph** | CSV export | Title, author, rating, moods, pace, dates, content warnings | 🟡 | Phase 3 | `[Validated]` — 23-column CSV, see `docs/validations/storygraph-export.md` |
| **Manual entry** | CLI flags | User-supplied fields | 🟢 | **MVP** | N/A |
| **ISBN list** | Plain text | ISBNs only → metadata fetch | 🟢 | **MVP** | N/A |
| **Generic CSV** | User-mapped CSV | Flexible column mapping | 🟡 | Phase 2 | N/A |
| **LibraryThing** | CSV/JSON | Full metadata, tags, ratings | 🟡 | Phase 3 | `[Validation Required]` |
| **BibTeX** | `.bib` files | Academic citations | 🟡 | Phase 3 | N/A |

**Goodreads import detail** — This is the MVP killer feature. The Goodreads CSV export `[Validation Required]` typically contains these columns: Book Id, Title, Author, Author l-f, Additional Authors, ISBN, ISBN13, My Rating, Average Rating, Publisher, Binding, Number of Pages, Year Published, Original Publication Year, Date Read, Date Added, Bookshelves, Bookshelves with positions, Exclusive Shelf, My Review, Spoiler, Private Notes, Read Count, Owned Copies.

**Mapping strategy**:

- `Exclusive Shelf` → Reading status (read, currently-reading, to-read)
- `Bookshelves` → User shelves/tags
- `My Rating` → Rating (multiply by 2 for 0–10 scale)
- `Date Read` → Reading session end date
- `Date Added` → Date added to library
- `My Review` → Review text
- `ISBN13` → Primary identifier for metadata enrichment
- `Book Id` → Stored as `goodreads_id` for dedup on re-import

### 3.1.1 — Import Fidelity & Migration UX

Every importer implements the `ImportEngine` trait with these capabilities:

- **Dry-run mode** 🟢: `toku import goodreads ~/export.csv --dry-run` → shows what would happen without writing.
- **Idempotent re-import** 🟡: Source ID stored per book (`goodreads_id`, `calibre_id`). Re-importing the same file skips existing entries, updates changed fields (respecting user edits).
- **Match confidence** 🟡:
  - **Exact**: ISBN match → auto-merge
  - **High**: Title + Author exact match → auto-merge with confirmation summary
  - **Fuzzy**: Partial title match → flagged for user review
  - **None**: New entry created
- **Partial failure handling** 🟡: Import progress checkpointed every 100 books. On failure, `toku import --resume` continues from last checkpoint.
- **Field mapping report** 🟢: Post-import summary showing fields imported, matched, skipped.
- **Provenance tracking** 🟢: Every imported field records `(source, date)`. User edits clear provenance and mark the field as `user_override`.
- **Rollback** 🟡: `toku import undo <import-id>` removes all books added by that import.
- **Import log** 🟢: `import_logs` table records every import operation.

### 3.1.2 — Canonical Lossless Export Format

**Recommendation**: ZIP archive containing JSON + cover images.

```text
toku-backup-2025-07-14.zip
├── manifest.json          # Version, export date, book count
├── library.json           # Full data model as structured JSON
├── covers/                # Cover images by content hash
│   ├── a1b2c3d4.jpg
│   └── ...
└── schema-version.txt     # Data model version for forward compatibility
```

- **Self-contained**: Single ZIP file, no external dependencies.
- **Documented**: JSON schema published in `docs/export-format.md`.
- **Versioned**: `schema-version.txt` enables future parsers to handle old exports.
- **Round-trip**: `toku export backup` → `toku import backup toku-backup.zip` = identical library.

Rejected alternatives:

- SQLite dump: portable but opaque — users can't inspect without SQLite tools.
- JSON-only: doesn't include cover images.

### 3.2 — Export System

| Format | Use Case | Phase | Complexity |
|--------|----------|-------|-----------|
| JSON (full) | Backup, programmatic access | **MVP** | 🟢 |
| CSV | Spreadsheet users | **MVP** | 🟢 |
| Markdown | Blog posts, reading lists | Phase 2 | 🟢 |
| ZIP backup (canonical) | Full backup/migration | Phase 2 | 🟡 |
| BibTeX | Academic citations | Phase 3 | 🟡 |
| OPDS | E-reader catalog | Phase 6 | 🟡 |
| HTML | Static reading list site | Phase 3 | 🟡 |

### 3.3 — Book Metadata Sources

**Primary: Open Library API** `[Validated]`

- Free, no API key required, community-maintained.
- REST API: `https://openlibrary.org/isbn/{isbn}.json`
- Good coverage for popular titles. Gaps for non-English, self-published, and niche academic works.
- Cover API: `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg`
- Rate limits: generous (100 req/min stated, practically higher) `[Validation Required]`

**Fallback: Google Books API** `[Validated — significant restrictions]`

- 1,000 requests/day free tier (no key), higher with API key.
- Broader coverage than Open Library, especially for recent releases.
- Terms of service: **thumbnails cannot be cached locally** (ToS Section 5e.1 prohibits permanent copies). Metadata caching is restricted to HTTP cache header TTL. API keys cannot be embedded in open-source projects (Section 4b.1). See `docs/validations/cover-image-licensing.md`.
- Google Books is viable only for transient text metadata enrichment (title, author, pages), not cover images.

**Merge strategy**: Query Open Library first. If no result or missing fields, query Google Books. User edits always take precedence. Empty fields filled from the highest-quality available source.

Rejected alternatives:

- ISBNdb: paid — violates $0 budget constraint.
- WorldCat: Search API terms unclear for open-source use `[Validation Required]`.
- BookBrainz: growing but sparse coverage.

### 3.4 — Cover Image Strategy

- **Sources**: Open Library Covers API (primary), user upload via `toku cover set <book> <path>`. Google Books thumbnails **cannot** be cached locally per Google's ToS — see `docs/validations/cover-image-licensing.md`.
- **Storage**: Filesystem, content-addressed. `covers/{sha256_first_16_chars}.jpg`. Thumbnails generated on first display (200px width) and cached as `covers/thumb/{hash}.jpg`.
- **Offline**: Covers fetched once from Open Library, stored locally. App works without covers (text-only display in CLI).
- **No cover placeholder**: Color-coded rectangle derived from genre/title hash. Visually distinct per book.
- **Resolution**: Store original as fetched (typically 300–600px). Generate thumbnails on demand.
- **Licensing**: Open Library on-demand caching is permitted per their API guidelines. Bulk crawling is prohibited. Courtesy attribution appreciated.

### 3.5 — Non-Goals & Red Lines

- ❌ **No piracy features**: No ebook downloading, no DRM stripping, no torrent integration.
- ❌ **No scraping**: No Goodreads web scraping (Goodreads has no public API since 2020), no Amazon scraping.
- ❌ **No write-back**: Import is one-directional. Toku does not update Goodreads, StoryGraph, or any external service.
- ❌ **No social features**: No user accounts for sharing, no activity feeds, no book clubs.
- ❌ **No cloud dependency for core features**: Metadata fetch is enrichment, not requirement.

---

## Section 3A: Bibliographic Domain Complexity & Canonical Modeling

### 3A.1 — Work / Edition / Copy Model

**Recommendation**: Start with **Book = Edition** (flat model), add Work grouping in Phase 3.

**MVP (Phases 0–2)**: Each book row represents a specific edition. The schema includes a nullable `work_id` column (UUID) that is NULL initially. When two books are the "same work" (e.g., hardcover and Kindle edition of Dune), they can be linked later.

**Phase 3 migration**: Introduce a `works` table. Run a migration that groups books by normalized title + primary author. Present ambiguous matches for user confirmation. This is a batch operation, not an ongoing workflow — most users with 100–500 books have few duplicate works.

**Why not full FRBR from day one?** FRBR (Functional Requirements for Bibliographic Records) is designed for libraries serving millions of patrons. For a personal library of 100–500 books, the work/expression/manifestation/item hierarchy adds UI complexity without matching value. A user adding "Dune" doesn't want to navigate a work tree — they want one book entry they can refine.

**How Goodreads handles this**: Goodreads conflates work and edition — a "book" page aggregates all editions, but reviews and ratings are per-work. This causes problems: the page count shown may not match the user's edition, and edition-specific metadata is lossy. Toku avoids this by making each entry an edition with optional work grouping.

### 3A.2 — Contributor Modeling

**Data model** (MVP):

```sql
contributors (id, name, sort_name, external_ids_json)
book_contributors (book_id, contributor_id, role, position)
```

**Roles** (enum): `author`, `co_author`, `editor`, `translator`, `illustrator`, `narrator`, `foreword`, `compiler`, `contributor`.

**Name handling**:

- `name`: Display name ("Ursula K. Le Guin")
- `sort_name`: Sort key ("Le Guin, Ursula K.")
- Pseudonyms: stored as separate contributor entries linked via `alias_of` nullable FK.
- "Various Authors" / "Anonymous": special-cased with a flag, not string matching.

**MVP simplification**: Most imported books have one author. The multi-contributor model exists in the schema but the CLI defaults to `--author "Name"` for the common case. Complex contributor entry is available via `toku contributor add` and flags.

### 3A.3 — Identifier Systems

| Identifier | Column | Validation | Notes |
|------------|--------|-----------|-------|
| ISBN-13 | `isbn13` (TEXT) | Check digit validation | Primary identifier |
| ISBN-10 | `isbn10` (TEXT) | Check digit, auto-convert to ISBN-13 | Legacy, stored alongside ISBN-13 |
| ASIN | `asin` (TEXT) | Format check (B0...) | Common for Kindle books |
| Goodreads ID | `goodreads_id` (TEXT) | Numeric string | Import dedup key |
| Open Library ID | `openlibrary_id` (TEXT) | /works/OL... or /editions/OL... | Metadata enrichment link |
| Internal UUID | `id` (TEXT, UUID v7) | Always present | Primary key, sortable by creation time |

**Books without identifiers**: Fully supported. Manual entry creates a book with only a UUID. Title + Author are the minimum fields. ISBNs are optional. The app prompts for ISBN enrichment but never requires it.

**Source-of-truth precedence**: User edit > Import source data > API-fetched metadata. Per-field provenance tracks this chain.

### 3A.4 — Edge Cases

| Edge Case | MVP Handling | Full Handling (Phase 3+) |
|-----------|-------------|------------------------|
| Anthologies | Single book entry, multiple authors via roles | Nested works within a parent book |
| Omnibus editions | Single book entry with note | Link to constituent works |
| Translations | Separate book entry per language | Linked as editions of same work |
| Revised editions | Separate entries | Linked as editions of same work |
| Audiobooks | Book entry with `format=audiobook`, duration field, narrator as contributor | Full audiobook metadata (chapters, bitrate) |
| Comics/manga | Book entry with `format=comic`, volume number in series | Artist/writer role distinction |
| Partial dates | Year-only stored as `YYYY-01-01` with `date_precision` flag | Same |
| Series complexity | `series_name` + `series_position` (decimal for sub-numbering) | Multiple series membership |
| No-ISBN books | Manual entry, title+author as dedup key | Same |

### 3A.5 — Metadata Correction & Provenance

Every mutable metadata field on a book has an associated provenance record:

```sql
metadata_provenance (
  book_id TEXT,
  field_name TEXT,
  source TEXT,        -- 'user', 'goodreads_import', 'openlibrary_api', etc.
  source_date TEXT,   -- ISO 8601
  is_user_override BOOLEAN DEFAULT FALSE,
  PRIMARY KEY (book_id, field_name)
)
```

**Rules**:

1. User edits set `is_user_override = TRUE`. Auto-enrichment skips fields where `is_user_override = TRUE`.
2. Re-import updates non-overridden fields with newer source data.
3. `toku book provenance "Dune"` shows the source of every field.

### 3A.6 — Internationalization

- **MVP**: English-only UI. Book metadata in any language/script (UTF-8 throughout).
- **Title handling**: Single `title` field. `original_title` as optional separate field for translations.
- **Sorting**: ICU collation via SQLite ICU extension `[Validation Required]` — needed for correct CJK, Arabic, Cyrillic sort order. Fallback: binary sort (acceptable for MVP).
- **App i18n**: Deferred to post-1.0. Design for it (externalize strings) but don't implement.

---

## Section 4: Core Feature Set

### 4.1 — Book Entry & Metadata Management

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Add book manually (title, author) | ✅ | ✅ | 🟢 |
| Add by ISBN with metadata fetch | ✅ | ✅ | 🟢 |
| Add by title/author search | ❌ | ✅ | 🟡 |
| Edit any metadata field | ✅ | ✅ | 🟢 |
| Custom user-defined fields | ❌ | ✅ (Phase 3) | 🟡 |
| Series management | ✅ (basic) | ✅ | 🟢 |
| Edition tracking (work grouping) | ❌ | ✅ (Phase 3) | 🟡 |
| Author page (all books by author) | ❌ | ✅ (Phase 2) | 🟢 |
| Cover image auto-fetch | ✅ | ✅ | 🟢 |
| Merge duplicate books | ❌ | ✅ (Phase 3) | 🟡 |

### 4.2 — Reading Status & Progress Tracking

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Reading states (want/reading/read/abandoned/on-hold) | ✅ | ✅ | 🟢 |
| Start and finish dates | ✅ | ✅ | 🟢 |
| Page-based progress | ❌ | ✅ (Phase 2) | 🟢 |
| Time-based progress (audiobooks) | ❌ | ✅ (Phase 2) | 🟡 |
| Multiple reading sessions (re-reads) | ❌ | ✅ (Phase 2) | 🟡 |
| Reading log entries | ❌ | ✅ (Phase 2) | 🟢 |
| Reading speed calculation | ❌ | ✅ (Phase 3) | 🟡 |
| Daily/weekly page targets | ❌ | ✅ (Phase 3) | 🟡 |

### 4.3 — Library Organization

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Shelves / collections | ✅ | ✅ | 🟢 |
| Tags (free-form) | ✅ | ✅ | 🟢 |
| Smart shelves (saved filters) | ❌ | ✅ (Phase 3) | 🟡 |
| Sorting (title, author, date, rating) | ✅ | ✅ | 🟢 |
| Filtering (status, tag, shelf) | ✅ | ✅ | 🟢 |

### 4.4 — Ratings, Reviews & Notes

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Star rating (0–10, displayed as 0–5 with halves) | ✅ | ✅ | 🟢 |
| Written review (markdown) | ✅ | ✅ | 🟢 |
| Per-book notes | ❌ | ✅ (Phase 2) | 🟢 |
| Mood tags | ❌ | ✅ (Phase 3) | 🟢 |
| Pace rating (fast/medium/slow) | ❌ | ✅ (Phase 3) | 🟢 |
| Content warnings (user-defined) | ❌ | ✅ (Phase 3) | 🟢 |

### 4.5 — Search & Discovery

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Full-text search (FTS5) | ✅ | ✅ | 🟢 |
| Fuzzy search (typo-tolerant) | ❌ | ✅ (Phase 3) | 🟡 |
| Faceted search (filter by status, tag, shelf) | ✅ (basic) | ✅ | 🟡 |
| Random book picker ("what should I read?") | ❌ | ✅ (Phase 2) | 🟢 |
| "More like this" | ❌ | ✅ (Phase 3) | 🟡 |

### 4.6 — Reading Statistics & Analytics

| Feature | MVP | Full | Complexity |
|---------|-----|------|-----------|
| Books read this year/month | ❌ | ✅ (Phase 2) | 🟢 |
| Reading pace (books/month) | ❌ | ✅ (Phase 2) | 🟢 |
| Genre/tag distribution | ❌ | ✅ (Phase 3) | 🟡 |
| Rating distribution | ❌ | ✅ (Phase 3) | 🟢 |
| Author diversity stats | ❌ | ✅ (Phase 3) | 🟡 |
| Reading streaks | ❌ | ✅ (Phase 3) | 🟡 |
| Time-to-finish averages | ❌ | ✅ (Phase 3) | 🟡 |
| Mood trends over time | ❌ | ✅ (Phase 3) | 🟡 |
| Format breakdown (physical/ebook/audio) | ❌ | ✅ (Phase 3) | 🟢 |
| Yearly wrap-up report | ❌ | ✅ (Phase 3) | 🔴 |
| Goal tracking (52 books/year) | ❌ | ✅ (Phase 3) | 🟡 |

All statistics computed locally from SQLite queries. No server needed.

### 4.7 — Reading Goals & Challenges

| Feature | Phase | Complexity |
|---------|-------|-----------|
| Annual book count goal | Phase 3 | 🟢 |
| Page count goal | Phase 3 | 🟢 |
| Custom challenges | Phase 3 | 🟡 |
| Challenge templates | Post-1.0 | 🟡 |

---

## Section 5: Features You May Have Missed

| # | Feature | Why It Matters | Phase | Complexity |
|---|---------|---------------|-------|-----------|
| 1 | **Lending tracker** | "Lent Dune to Alex on March 5 — remind me in 30 days." Tracks who has your books. | Phase 2 | 🟢 |
| 2 | **Quotes / highlights** | Save favorite passages with page refs. High-value for re-reads. | Phase 2 | 🟢 |
| 3 | **TBR priority ranking** | Numbered priority for the to-read pile. `toku tbr next` returns the top pick. | Phase 2 | 🟢 |
| 4 | **Reading journal** | Date-stamped free-form entries while reading — not reviews, but in-progress thoughts. | Phase 3 | 🟢 |
| 5 | **Book timeline** | Visual timeline: when did I read what? Rendered as ASCII art in CLI, SVG in web. | Phase 3 | 🟡 |
| 6 | **DNF tracking** | "Stopped at page 142 because the pacing fell apart." Abandoned status + reason + page. | Phase 2 | 🟢 |
| 7 | **Batch operations** | `toku tag add "sci-fi" --shelf "2024 Reads"` — tag 50 books in one command. | Phase 2 | 🟡 |
| 8 | **Data integrity checks** | `toku doctor` — find orphaned records, missing covers, invalid ISBNs. | Phase 2 | 🟡 |
| 9 | **Reading time estimation** | "Based on your average speed, this 400-page book will take ~8 hours." | Phase 3 | 🟡 |
| 10 | **Location tracking** | "Bedroom shelf, row 2." Physical book locations. Simple text field per book. | Phase 2 | 🟢 |
| 11 | **Re-read tracking with separate ratings** | Different rating per read. "I rated Dune 4★ in 2019 and 5★ on re-read in 2024." | Phase 2 | 🟡 |
| 12 | **Library value tracking** | Purchase price field. Total library value report for collectors/insurance. | Phase 3 | 🟢 |
| 13 | **Barcode/ISBN scanning** | Camera-based ISBN scan on mobile. Key feature for iOS app. | Phase 5 | 🟡 |
| 14 | **Dark mode / accessibility** | All interfaces respect system theme. Screen reader support. `NO_COLOR` env var. | Phase 1+ | 🟢 |
| 15 | **Shell completions** | bash, zsh, fish, PowerShell completions auto-generated from clap. | Phase 1 | 🟢 |

---

## Section 6: Ebook File Management (Future — Phase 6)

**Recommendation**: Defer file management to Phase 6 (post-1.0). The MVP is a reading tracker, not a file manager. The architecture accommodates file management through the modular crate design, but the feature set is not committed.

### 6.1 — File Management (Phase 6)

- Associate ebook files (.epub, .pdf, .mobi, .azw3) with book entries 🟡
- Organize files on disk using configurable templates (`{author}/{title}.{format}`) 🟡
- Multiple formats per book 🟢
- File integrity checking (SHA-256 checksums) 🟢
- Disk usage reporting 🟢

### 6.2 — Format Conversion (Phase 6)

**Recommendation**: Shell out to Calibre's `ebook-convert` CLI tool.

Building a format converter in Rust is a multi-month project. Calibre's converter is mature, handles edge cases, and is free. The dependency is optional — `toku convert` checks for `ebook-convert` in `$PATH` and provides installation instructions if missing. 🟡

DRM note: Only DRM-free files are supported. No DRM stripping.

### 6.3 — OPDS Server (Phase 6)

- Serve the library as an OPDS catalog for e-readers (KOReader, Moon+ Reader) 🟡
- Local network only by default
- Optional basic authentication

### 6.4 — Architecture Note

File management lives in a new `toku-files` crate that depends on `toku-core` and `toku-db`. It adds file path columns to the book table and a `files` table for multi-format tracking. This does NOT affect the metadata sync strategy — file sync is a separate, harder problem deferred to Phase 7.

---

## Section 7: Moonshot Features

### 7.1 — AI-Powered Personal Recommendations 🔴

- On-device only. No cloud, no data sharing.
- Approach: TF-IDF or lightweight embeddings over book descriptions + user ratings. Not deep learning.
- Feasibility: A Rust-native recommendation engine using `ndarray` + cosine similarity over user taste vectors is viable for 500 books. Full ML is overkill.
- Phase: Post-1.0 research.

### 7.2 — Reading Session Timer 🟡

- `toku timer start "Dune"` → `toku timer stop` → auto-log pages read and duration.
- Useful for reading speed statistics.
- Phase: Phase 3 (simple), Phase 5 for mobile widget.

### 7.3 — Web Clipper / Article Saver ⛔

- **Recommendation: Out of scope.** This overlaps with Pocket/Omnivore/Readwise. Adding it dilutes the book focus. Users who want article tracking should use a dedicated tool.

### 7.4 — Ebook Reader Integration 🔴

- Kindle Clippings.txt: parseable, well-documented format `[Validation Required]`. Import highlights/notes.
- Kobo SQLite DB: `KoboReader.sqlite` on device `[Validation Required]`. Reading progress sync.
- Apple Books: annotations in `~/Library/Containers/com.apple.iBooksX/` `[Validation Required]`.
- Phase: Post-1.0 research. High value but each reader is a separate integration effort.

### 7.5 — Self-Hosted Sync Server 🔴

- Deferred to Phase 7. See ADR-010 for the current architecture decision.
- Immich-style self-hostable Docker image with first-run admin onboarding, real user
  accounts, and 1Password-style two-secret SRP authentication.
- Mandatory zero-knowledge E2E encryption — no plaintext mode for hosted sync.
- Axum server with REST API (op-log wire protocol from ADR-008 retained).
- cr-sqlite kept as a research alternative if it matures for iOS/WASM.

---

## Section 8: Data Model

### Entity Relationship Diagram (Text)

```text
works (1) ──── (M) books
books (1) ──── (M) book_contributors ──── (M) contributors
books (1) ──── (M) book_series ──── (M) series
books (1) ──── (M) reading_sessions
books (1) ──── (M) reading_progress
books (1) ──── (M) book_shelves ──── (M) shelves
books (1) ──── (M) book_tags ──── (M) tags
books (1) ──── (M) reviews
books (1) ──── (M) notes
books (1) ──── (M) custom_fields
books (1) ──── (M) identifiers
books (1) ──── (1) cover_images
books (1) ──── (M) metadata_provenance
import_logs (standalone)
reading_goals (standalone)
```

### Core Tables

```sql
-- Books (each row is an edition)
CREATE TABLE books (
  id TEXT PRIMARY KEY,              -- UUID v7
  work_id TEXT,                      -- nullable FK to works (Phase 3)
  title TEXT NOT NULL,
  subtitle TEXT,
  description TEXT,
  page_count INTEGER,
  duration_minutes INTEGER,          -- audiobooks: total duration
  publication_date TEXT,             -- ISO 8601 (YYYY, YYYY-MM, or YYYY-MM-DD)
  date_precision TEXT DEFAULT 'day', -- 'year', 'month', 'day'
  language TEXT,                     -- ISO 639-1
  format TEXT DEFAULT 'physical',    -- physical, ebook, audiobook, comic
  publisher TEXT,
  cover_hash TEXT,                   -- references covers/{hash}.jpg
  status TEXT DEFAULT 'want_to_read', -- want_to_read, reading, read, abandoned, on_hold
  rating INTEGER,                    -- 0-10 (displayed as 0-5 stars with halves)
  date_added TEXT NOT NULL,          -- ISO 8601
  date_started TEXT,
  date_finished TEXT,
  location TEXT,                     -- physical location ("bedroom shelf, row 2")
  purchase_price REAL,              -- optional, for collectors
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- Contributors (authors, translators, narrators, etc.)
CREATE TABLE contributors (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  sort_name TEXT,
  alias_of TEXT,                     -- FK to contributors.id for pseudonyms
  bio TEXT,
  birth_year INTEGER,
  death_year INTEGER,
  external_ids TEXT                  -- JSON: {"openlibrary": "OL...", "isni": "..."}
);

CREATE TABLE book_contributors (
  book_id TEXT NOT NULL REFERENCES books(id),
  contributor_id TEXT NOT NULL REFERENCES contributors(id),
  role TEXT NOT NULL DEFAULT 'author',
  position INTEGER NOT NULL DEFAULT 0,  -- display order
  PRIMARY KEY (book_id, contributor_id, role)
);

-- Identifiers
CREATE TABLE identifiers (
  book_id TEXT NOT NULL REFERENCES books(id),
  id_type TEXT NOT NULL,             -- isbn13, isbn10, asin, goodreads_id, openlibrary_id, etc.
  id_value TEXT NOT NULL,
  PRIMARY KEY (book_id, id_type, id_value)
);

-- Series
CREATE TABLE series (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  total_books INTEGER               -- NULL if unknown
);

CREATE TABLE book_series (
  book_id TEXT NOT NULL REFERENCES books(id),
  series_id TEXT NOT NULL REFERENCES series(id),
  position REAL,                     -- decimal for sub-numbering (1.5 for novellas)
  PRIMARY KEY (book_id, series_id)
);

-- Reading sessions (supports re-reads)
CREATE TABLE reading_sessions (
  id TEXT PRIMARY KEY,
  book_id TEXT NOT NULL REFERENCES books(id),
  start_date TEXT,
  end_date TEXT,
  status TEXT NOT NULL,              -- reading, read, abandoned, on_hold
  rating INTEGER,                    -- per-session rating (may differ from book rating)
  review_text TEXT,
  created_at TEXT NOT NULL
);

-- Reading progress log
CREATE TABLE reading_progress (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES reading_sessions(id),
  logged_at TEXT NOT NULL,
  page_number INTEGER,
  percentage REAL,
  duration_minutes INTEGER,          -- for audiobooks: minutes listened
  notes TEXT
);

-- Shelves and tags
CREATE TABLE shelves (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT,
  is_smart BOOLEAN DEFAULT FALSE,
  smart_filter TEXT                  -- JSON filter definition for smart shelves
);

CREATE TABLE book_shelves (
  book_id TEXT NOT NULL REFERENCES books(id),
  shelf_id TEXT NOT NULL REFERENCES shelves(id),
  PRIMARY KEY (book_id, shelf_id)
);

CREATE TABLE tags (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  tag_type TEXT DEFAULT 'general'    -- general, mood, pace, content_warning
);

CREATE TABLE book_tags (
  book_id TEXT NOT NULL REFERENCES books(id),
  tag_id TEXT NOT NULL REFERENCES tags(id),
  PRIMARY KEY (book_id, tag_id)
);

-- Reviews and notes
CREATE TABLE reviews (
  id TEXT PRIMARY KEY,
  book_id TEXT NOT NULL REFERENCES books(id),
  session_id TEXT REFERENCES reading_sessions(id),
  rating INTEGER,
  text TEXT,
  date_written TEXT NOT NULL,
  updated_at TEXT
);

CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  book_id TEXT NOT NULL REFERENCES books(id),
  page_reference TEXT,               -- page number, chapter, or timestamp
  text TEXT NOT NULL,
  note_type TEXT DEFAULT 'note',     -- note, quote, highlight
  created_at TEXT NOT NULL
);

-- Custom fields (key-value per book)
CREATE TABLE custom_fields (
  book_id TEXT NOT NULL REFERENCES books(id),
  field_name TEXT NOT NULL,
  field_value TEXT NOT NULL,
  PRIMARY KEY (book_id, field_name)
);

-- Reading goals
CREATE TABLE reading_goals (
  id TEXT PRIMARY KEY,
  year INTEGER NOT NULL,
  goal_type TEXT NOT NULL,           -- books, pages
  target INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

-- Metadata provenance
CREATE TABLE metadata_provenance (
  book_id TEXT NOT NULL REFERENCES books(id),
  field_name TEXT NOT NULL,
  source TEXT NOT NULL,
  source_date TEXT NOT NULL,
  is_user_override BOOLEAN DEFAULT FALSE,
  PRIMARY KEY (book_id, field_name)
);

-- Import logs
CREATE TABLE import_logs (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  file_path TEXT,
  imported_at TEXT NOT NULL,
  total_records INTEGER,
  imported_count INTEGER,
  skipped_count INTEGER,
  error_count INTEGER,
  errors_json TEXT,
  status TEXT DEFAULT 'completed'    -- in_progress, completed, failed, rolled_back
);

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE books_fts USING fts5(
  title, subtitle, description,
  content='books', content_rowid='rowid'
);
```

**Audiobook support**: The `books` table includes `duration_minutes` for total audiobook length. `reading_progress` includes `duration_minutes` for per-session listening time. The `contributor` role `narrator` handles audiobook narrators. This avoids a separate audiobook table.

**Custom fields**: Implemented as a key-value join table — no schema changes per field. Queryable via `SELECT * FROM custom_fields WHERE field_name = 'course'`.

**Migration strategy**: `refinery` crate manages versioned migrations (`V001__initial_schema.sql`, `V002__add_work_id.sql`, etc.). Migrations run automatically on startup. Schema version stored in SQLite `PRAGMA user_version`.

---

## Section 9: Design Brief

### 9.1 — Design Principles

1. **Personal & Private** — No login screens, no accounts, no "sign in to continue." The app opens directly to your library.
2. **Information-Dense** — Show maximum useful data per screen. Terminal real estate is precious. Avoid sparse layouts.
3. **Keyboard-First** — Every action reachable via keyboard. CLI is the primary interface. Web UI uses keyboard shortcuts.
4. **Your Data, Readable** — The underlying SQLite database is a feature, not an implementation detail. Users can query it directly with `sqlite3`.
5. **Fast & Responsive** — <200ms CLI startup. Instant search. No loading spinners for local operations.
6. **Graceful Degradation** — Works without network (no metadata fetch). Works without covers (text mode). Works on a 80-column terminal.

### 9.2 — CLI Design

**Command structure**:

```bash
# Book management
toku add "Dune" --author "Frank Herbert"
toku add --isbn 9780441013593
toku edit "Dune" --rating 9 --tag "sci-fi" --tag "classic"
toku show "Dune"                          # detailed view
toku list                                  # all books, default sort
toku list --status reading --sort progress
toku search "herbert"                      # full-text search
toku remove "Dune" --confirm

# Reading tracking
toku start "Dune"                          # set status to 'reading'
toku update "Dune" --page 145              # log progress
toku finish "Dune" --rating 9              # mark as read
toku abandon "Dune" --page 87 --reason "Too slow"

# Organization
toku shelf create "Beach Reads"
toku shelf add "Beach Reads" "Dune" "Neuromancer"
toku tag add "sci-fi" "Dune" "Neuromancer" "Snow Crash"

# Import / Export
toku import goodreads ~/goodreads_export.csv
toku import goodreads ~/goodreads_export.csv --dry-run
toku import calibre ~/Calibre\ Library/
toku export csv --output ~/my-library.csv
toku export json --output ~/library.json
toku export backup --output ~/toku-backup.zip

# Statistics
toku stats                                 # current year summary
toku stats --year 2024
toku stats --author "Frank Herbert"

# Utilities
toku doctor                                # data integrity check
toku config                                # show/edit configuration
toku enrich --isbn-only                    # fetch metadata for books with ISBNs but missing data
```

**Output formatting**:

- Default: rich terminal tables with color (via `tabled` or `comfy-table` crate)
- `--format json`: machine-readable JSON output
- `--format csv`: CSV output
- `--format markdown`: markdown table
- `--no-color` / `NO_COLOR` env var: plain text

**CLI framework**: `clap` v4 with derive macros. Auto-generates help, shell completions (bash/zsh/fish/PowerShell), and man pages.

**Configuration**: TOML file at `{data_dir}/config.toml`:

```toml
[display]
default_format = "table"
color = true
date_format = "%Y-%m-%d"

[metadata]
auto_fetch = true
primary_source = "openlibrary"
fallback_source = "google_books"

[library]
default_status = "want_to_read"
```

### 9.3 — Web UI Design (Phase 4) ✅

- **Technology**: Axum backend with maud for server-side HTML rendering (see ADR-007).
  Inline SVG charts, CSS custom properties with `prefers-color-scheme` dark mode.
  Minimal inline JavaScript (~15 lines for SSE import progress only).
- **Key screens**: Library grid/list with cover images, book detail with reading timeline,
  statistics dashboard with rating histogram / monthly pace / format breakdown / top authors,
  import wizard with dry-run preview and live SSE progress, FTS5-powered search.
- **Dark mode**: Automatic via `prefers-color-scheme` CSS media query.
- **Fully offline**: No CDN, no npm, no external assets. Works on `localhost` with no internet.
- **Responsive**: Pagination (60 books/page), grid/list toggle, sort controls.

### 9.4 — Key Screens Per Platform

| Platform | Key Screens |
|----------|------------|
| **CLI** | `list` (table), `show` (detail), `stats` (summary), `search` (results), `import` (progress) |
| **Web** | Library grid, book detail + progress, stats dashboard with charts, import wizard, search with facets |
| **iOS** | Library grid, book detail, quick progress update, barcode scanner, stats glance |
| **macOS** | Sidebar + content, multi-column library, stats with charts |

### 9.5 — Information Architecture

- **CLI navigation**: Command-based. `toku list` → `toku show "Dune"` → `toku edit "Dune" --rating 9`. No stateful navigation — each command is independent.
- **Search UX**: `toku search` uses FTS5. Instant results. Filters via flags (`--status`, `--tag`, `--shelf`).
- **Onboarding**: First run detects empty library → suggests `toku import goodreads <file>` or `toku add --isbn <isbn>`. Interactive welcome message.

---

## Section 10: Competitive Analysis

### 10.1 — Direct Competitors

| App | Privacy | Platforms | Export | Offline | Open Source | Key Weakness |
|-----|---------|-----------|--------|---------|-------------|-------------|
| **Goodreads** | Poor (Amazon) | Web, mobile | CSV (lossy) | No | No | Stagnant UI, Amazon data harvesting, declining API |
| **StoryGraph** | Moderate | Web, mobile | CSV | No | No | No offline mode, limited customization, no CLI |
| **Bookwyrm** | Good (federated) | Web | JSON-LD | Self-host only | Yes (AGPL) | Social-first (opposite philosophy), complex setup |
| **Hardcover** | Moderate | Web | API | No | Partially | Young platform, limited export |
| **Oku** | Good | Web | Limited | No | Yes | Minimal features, small community |
| **BookTracker** | Good (local) | iOS only | None | Yes | No | Apple-only, no CLI, no export |

### 10.2 — Library Managers

| App | Strengths | Weaknesses for Our User |
|-----|-----------|------------------------|
| **Calibre** | Unmatched metadata engine, format conversion, plugin ecosystem | No reading tracking, dated UI, desktop-only |
| **Kavita** | Self-hosted reading server, good UI | Focused on serving files, not tracking reading |
| **Audiobookshelf** | Excellent audiobook management | Audiobook-specific, not general book tracking |

### 10.3 — Differentiation Matrix

```text
                    High Data Ownership
                          │
              Toku ●      │
                          │      ● Calibre
                          │
    Minimal ──────────────┼────────────── Feature Rich
    Features              │               Features
                          │
              ● Oku       │      ● StoryGraph
                          │      ● Goodreads
                          │
                    Low Data Ownership
```

**Why Toku wins now**:

1. **Goodreads is rotting**: No API since 2020, UI unchanged for years, Amazon integration concerns growing.
2. **Privacy wave**: Post-GDPR, readers increasingly care about data ownership.
3. **CLI renaissance**: Tools like `ripgrep`, `fd`, `bat`, `jq` prove terminal-first tools find audiences.
4. **No competitor combines**: Calibre-grade metadata + StoryGraph-grade analytics + offline-first + open source + CLI.

---

## Section 11: Licensing & Open Source Strategy

### 11.1 — License Selection

#### Recommendation: MIT License

| License | Verdict | Reasoning |
|---------|---------|-----------|
| **MIT** ✅ | **Selected** | Maximum contributor friendliness. Standard in Rust ecosystem. No friction for forks, integrations, or commercial use. |
| Apache 2.0 | Runner-up | Patent grant is nice but adds legal complexity contributors don't expect in a book manager. |
| GPL v3 | Rejected | Copyleft friction reduces contributions. FFI consumers (iOS app) face licensing complexity. |
| AGPL v3 | Rejected | Strongest copyleft — overkill. Deters casual contributors. |
| Dual MIT+Apache | Rejected | Unnecessary complexity for a personal tool. |

### 11.2 — Contribution Model

- **DCO (Developer Certificate of Origin)** — recommended over CLA. Lightweight: contributors sign off commits with `Signed-off-by`. No legal entity needed.
- **CONTRIBUTING.md**: Code style (rustfmt + clippy), PR process, test requirements.
- **Issue templates**: Bug report, feature request, importer request, documentation improvement.
- **"Good first issue" strategy**: Label importer edge cases, documentation gaps, and CLI output improvements. These are self-contained and well-scoped.
- **Code of Conduct**: Contributor Covenant v2.1.

### 11.3 — Monetization & Sustainability

**Recommendation: GitHub Sponsors + voluntary donations.**

This is a personal tool, not a SaaS. There is no natural paywall that doesn't betray the open-source philosophy. GitHub Sponsors is the lowest-friction option: no infrastructure, no premium features to maintain, no user segmentation.

Rejected alternatives:

- Premium features: What feature would be premium? Every core feature must be free (principles 1–3).
- Hosted sync server: Possible post-1.0, but premature to plan.
- Open Collective: viable alternative, but GitHub Sponsors has better discoverability.

---

## Section 12: Tech Stack Recommendations

| Component | Choice | Justification |
|-----------|--------|--------------|
| **Core language** | **Rust** | User-confirmed. Excellent CLI ecosystem, cross-compilation, WASM, C FFI. `clap` is best-in-class. |
| **CLI framework** | **clap v4** (derive) | Industry standard in Rust. Auto-completions, man pages, typed arguments. |
| **Database** | **SQLite via rusqlite** | Battle-tested, single-file, FTS5, cross-platform. `rusqlite` is mature with bundled SQLite. |
| **Migrations** | **refinery** | Embedded SQL migrations, version tracking, type-safe. |
| **HTTP client** | **reqwest** | Async, well-maintained, TLS support. Used for metadata API calls. |
| **JSON** | **serde + serde_json** | De facto standard in Rust. |
| **CSV** | **csv crate** | Fast, streaming, handles Goodreads CSV edge cases (quoted fields, newlines in reviews). |
| **TOML** | **toml crate** | Config file parsing. |
| **CLI output** | **tabled** or **comfy-table** | Terminal table formatting with color support. |
| **UUID** | **uuid v7** (time-sortable) | Sortable by creation time, globally unique. |
| **Async runtime** | **tokio** (minimal features) | Only for HTTP requests. Core library is sync. |
| **Testing** | **Built-in + insta** (snapshot) | Snapshot tests for CLI output. Property-based tests via `proptest` for ISBN validation. |
| **CI/CD** | **GitHub Actions** | Multi-platform matrix (Linux, macOS, Windows). Release binaries via `cargo-dist`. |
| **Documentation** | **mdBook** (user docs) + **rustdoc** (API docs) | mdBook for the user guide. Rustdoc for crate documentation. |
| **Distribution** | **cargo install** + **GitHub Releases** (pre-built binaries) | `cargo-dist` automates cross-compilation and release. Homebrew tap in Phase 2. |

**Web stack (Phase 4)**: **Axum** (backend) + **maud** (server-side rendering). See ADR-007 for the decision against HTMX and Leptos. Inline SVG charts, no external JavaScript dependencies.

**iOS/macOS (Phase 5)**: **SwiftUI** consuming Rust core via C FFI (`toku-ffi` crate using `cbindgen`). Rejected React Native / Tauri for iOS due to UX quality concerns.

**Windows (Phase 5)**: **Tauri v2** — wraps the web UI in a native window. Avoids building a separate WinUI app.

### 12.1 — Proof-of-Concept Gates (Weeks 1–2)

Before committing to the architecture, validate:

| Gate | What to Prove | Success Criteria |
|------|--------------|-----------------|
| SQLite + FTS5 | Full-text search performance | <100ms query on 10,000 books |
| Goodreads CSV | Parse a real export | All documented fields extracted correctly |
| Open Library API | ISBN lookup + cover fetch | Successful lookup for 10 ISBNs, cover images downloaded |
| CLI output | Rich table formatting | Responsive to terminal width, color support, JSON fallback |
| Cargo workspace | Multi-crate build | `toku-core` compiles independently, `toku-cli` depends on all crates |
| Cross-compilation | Linux + macOS + Windows | CI builds pass on all three platforms |

---

## Section 13: Phased Roadmap with Milestones

### Phase 0: First Book ✅

**Theme**: "One book, stored and retrieved."
**Goal**: Add one book from the CLI, store it in SQLite, retrieve it. Validate the entire stack.

**Deliverables** (3):

1. Cargo workspace with `toku-core`, `toku-db`, `toku-cli` crates
2. `toku add --isbn <isbn>` → fetch metadata from Open Library → store in SQLite
3. `toku show <title>` and `toku list` → display book details

**Acceptance criteria**:

- `toku add --isbn 9780441013593` fetches "Dune" metadata and stores it
- `toku show "Dune"` displays title, author, ISBN, page count, cover status
- `toku list` displays a formatted table of all books
- All three work offline (with manually entered data, no fetch)

**Definition of done**: The developer can add, view, and list books. The data model is validated.

**Cut line**: Cover image display in CLI (text-only is fine for Phase 0).

---

### Phase 1: Minimum Usable Library (MVP) ✅

**Theme**: "My Goodreads library is here. I use this daily."
**Goal**: Import a full Goodreads library, track reading status, search and organize.

**Deliverables** (5):

1. **Goodreads CSV import** with dry-run, dedup, field mapping report
2. **Reading status management** (`toku start`, `toku finish`, `toku abandon`)
3. **Shelves and tags** (`toku shelf create`, `toku tag add`)
4. **Full-text search** (`toku search` with FTS5)
5. **Configuration file** + shell completions + `--format json` output

**Acceptance criteria**:

- Import 300 books from Goodreads CSV in <10 seconds, >95% field accuracy
- `toku start "Dune"` → `toku finish "Dune" --rating 9` works end-to-end
- `toku search "sci-fi"` returns relevant results in <100ms
- `toku import goodreads --dry-run` shows import preview without writing

**Dependencies**: Phase 0 complete.

**Risks**: Goodreads CSV format instability. **Mitigation**: test against multiple real exports, store raw CSV alongside parsed data.

**Cut line**: Cover image auto-fetch (can be Phase 2). Metadata enrichment for imported books (ISBNs are in the CSV — enrichment can happen later).

**Definition of done**: The developer has migrated their Goodreads library and uses `toku` daily for 2+ weeks.

---

### Phase 2: Reading Tracker ✅

**Theme**: "I track my reading progress and can see my history."
**Goal**: Page-by-page progress tracking, reading sessions (including re-reads), basic statistics, Calibre import, export.

**Deliverables** (5):

1. **Reading progress logging** (`toku update "Dune" --page 145`) with session tracking
2. **Re-read support** with separate sessions and per-session ratings
3. **Basic statistics** (`toku stats` — books read this year, reading pace, format breakdown)
4. **Calibre import** (parse `metadata.db`, import books + metadata + covers)
5. **Export** (CSV, JSON, canonical ZIP backup)

**Acceptance criteria**:

- `toku update "Dune" --page 145` logs progress with timestamp
- `toku stats --year 2025` shows books read, pages read, average rating
- Calibre import handles 1,000+ book library with covers
- `toku export backup` → `toku import backup` round-trips perfectly

**Dependencies**: Phase 1 complete.

**Cut line**: Audiobook duration-based progress (page-based only in Phase 2).

---

### Phase 3: Analytics & Polish (1.0 Release) ✅

**Theme**: "This is genuinely better than Goodreads for personal use."
**Goal**: Full statistics, mood/pace tags, goals, work grouping, smart shelves, StoryGraph import.

**Deliverables** (5):

1. **Full statistics engine** (genre distribution, rating histogram, reading streaks, mood trends, yearly wrap-up)
2. **Mood tags, pace ratings, content warnings**
3. **Work grouping** (link editions of the same book) + merge duplicates
4. **Smart shelves** (saved filter rules that auto-populate)
5. **StoryGraph import** `[Validated]` — see `docs/validations/storygraph-export.md`

**Acceptance criteria**:

- `toku stats --year 2024` produces a comprehensive report rivaling StoryGraph
- Mood tags from StoryGraph import are preserved
- Smart shelf "Unread sci-fi over 300 pages" auto-updates when new books match

**Cut line**: Custom challenges (templates can wait). Fuzzy search (FTS5 is good enough).

---

### Phase 4: Web Interface ✅

**Theme**: "Non-CLI users can use Toku."
**Goal**: Web UI for library browsing, book management, statistics, and import.

**Deliverables** (4):

1. Axum web server serving the library as a web app
2. Library grid/list view with search and filters
3. Statistics dashboard with charts
4. Import wizard (Goodreads, Calibre)

**Dependencies**: Phase 3 (stable data model). Web framework decision (Axum + HTMX vs Leptos).

---

### Phase 5: Native Apps ✅

**Theme**: "Toku on every device."

**Deliverables** (3):

1. iOS app (SwiftUI) with barcode scanning
2. macOS app (SwiftUI)
3. Windows app (Tauri v2 wrapping web UI)

**Dependencies**: Phase 4 (web UI provides the Tauri shell). `toku-ffi` crate for Swift bindings.

---

### Phase 6: File Management 🔴

**Theme**: "Calibre-grade ebook management."
**Deliverables**: File association, disk organization, format conversion (via Calibre), OPDS server.

---

### Phase 7: Sync & Multi-Device 🟡

**Theme**: "My library everywhere."
**Goal**: Opt-in multi-device sync via a lightweight changeset-based REST API. Users can sync between CLI, iOS, macOS, web, and Windows — without sacrificing the local-first guarantee. Sync is additive: a user who never enables sync loses nothing.

**Architecture**: See Section 2.5 (Sync Strategy) and ADR-010 for the current design. Summary: append-only op-log synced over REST with mandatory zero-knowledge E2E encryption (AES-256-GCM), 1Password-style SRP authentication, and Immich-style self-hosted Docker deployment.

**Deliverables** (5):

1. **Sync data model + op-log**: Local `sync_ops` table with Hybrid Logical Clock (HLC) timestamps, op IDs (UUID v7), entity type/ID, and encrypted-or-plaintext payload. Every mutation writes to both the domain table and the op-log in a single transaction.
2. **Sync server (`toku-sync` crate)**: Axum REST API for push/pull/snapshot/device management. Thin relay — stores ops and cursors, does not interpret content. Deployable as a Docker image (`kafkade/toku-sync`).
3. **Push/pull protocol with entity-specific merge rules**: Push sends new ops since last cursor. Pull receives ops and applies entity-specific merge: LWW per field for books, append-only for reading sessions, monotonic for progress, LWW with conflict detection for notes/reviews. Soft deletes with 30-day tombstone retention.
4. **Mandatory client-side E2E encryption + SRP auth**: Mandatory for hosted mode — no plaintext fallback. 1Password-style two-secret SRP (password + Secret Key). Key hierarchy: (Secret Key + password) → Argon2id → unlock key → wraps key pair → wraps library key. Emergency Kit generated at account creation.
5. **CLI sync commands + device management**: `toku sync init`, `toku sync push`, `toku sync pull`, `toku sync status`, `toku sync devices`. Device registration with UUID + human-readable name.

**Acceptance criteria**:

- Two devices (e.g., CLI on laptop + iOS app) can sync a library through the server with no data loss
- Offline edits on both devices merge correctly when both push/pull
- A deleted book on device A does not reappear on device B after sync
- A new device can bootstrap from a server snapshot + subsequent ops
- With encryption enabled, the server cannot decrypt any book content (verified by inspecting server storage)
- Sync resumes correctly after network failure (idempotent push/pull)
- `toku sync status` shows last sync time, pending ops count, and device list

**Dependencies**: Phase 3 (stable data model). Phase 4/5 (clients exist to sync between).

**Risks**:

- Schema migration during sync — ops from different app versions must coexist. **Mitigation**: version field in op envelope, backward-compatible op format.
- Conflict resolution UX — what happens when two devices edit the same book's title? **Mitigation**: LWW per field is invisible to the user in most cases; only note/review conflicts surface for manual resolution.
- Encryption complexity — nonce management, key storage, rotation. **Mitigation**: dedicated security review before merge; use well-audited crates (ring, aes-gcm).

**Cut line**: Managed hosted instance (`sync.toku.dev`) — self-hosted first. Browser-local SQLite for web app (web remains a connected companion). File sync for ebooks (Phase 6 files are not synced in Phase 7).

---

### Phase 8: Moonshots 🔴

**Theme**: "Beyond tracking."
**Deliverables**: On-device recommendations, ebook reader integration, reading timer.

---

## Section 14: First 90 Days — Execution Plan

### 14.1 — Technical Spikes (Weeks 1–2)

| Spike | Purpose | Success? |
|-------|---------|----------|
| Cargo workspace + `toku-core` model | Validate layered architecture | Core compiles independently |
| SQLite + FTS5 via `rusqlite` | Validate search performance | <100ms on 10k books |
| Goodreads CSV parse | Validate import fidelity | All fields from a real export mapped |
| Open Library API call | Validate metadata fetch | ISBN lookup + cover download works |
| clap CLI with `tabled` output | Validate CLI UX | Formatted table responsive to terminal width |
| GitHub Actions CI | Validate cross-platform builds | Linux + macOS + Windows green |

### 14.2 — First 10 Epics

| # | Epic | Acceptance Criteria | Effort | Dependency | Complexity |
|---|------|-------------------|--------|-----------|-----------|
| 1 | Project scaffold | Cargo workspace, CI, README, LICENSE, linting | S | None | 🟢 |
| 2 | Book model + SQLite persistence | Add book → store → retrieve round-trip | M | Epic 1 | 🟢 |
| 3 | Open Library metadata fetch | `add --isbn` fetches and stores metadata | M | Epic 2 | 🟢 |
| 4 | CLI: list, show, search | `toku list`, `toku show`, `toku search` with FTS5 | M | Epic 2 | 🟢 |
| 5 | Goodreads CSV import | Import 300+ books, dedup, dry-run, field mapping report | L | Epic 2 | 🟡 |
| 6 | Reading status management | `start`, `finish`, `abandon`, status transitions | M | Epic 2 | 🟢 |
| 7 | Shelves and tags | Create shelves, tag books, filter by shelf/tag | M | Epic 2 | 🟢 |
| 8 | Cover image pipeline | Fetch covers, store on disk, display status in CLI | M | Epic 3 | 🟡 |
| 9 | Configuration + shell completions | `config.toml`, bash/zsh/fish completions | S | Epic 4 | 🟢 |
| 10 | Export (CSV, JSON) | `toku export csv`, `toku export json` | S | Epic 2 | 🟢 |

### 14.3 — Due Diligence Backlog

| Task | Priority | Status |
|------|----------|--------|
| Download real Goodreads CSV export, document all columns | P0 | `[Validation Required]` |
| Test Open Library API: rate limits, non-English books, old editions | P0 | `[Validation Required]` |
| Google Books API: ToS for open-source, quota limits | P1 | `[Validated]` — significant ToS restrictions. See `docs/validations/cover-image-licensing.md` |
| StoryGraph: verify export availability and format | P1 | `[Validated]` — 23-column CSV export available. See `docs/validations/storygraph-export.md` |
| Calibre `metadata.db` schema: document all useful tables | P1 | `[Validated]` — schema is stable |
| ISBN-10 ↔ ISBN-13 conversion: verify algorithm, edge cases | P0 | `[Validated]` — well-documented standard |
| Cover image licensing: caching Open Library covers locally | P1 | `[Validated]` — on-demand caching permitted. See `docs/validations/cover-image-licensing.md` |
| `crates.io` name availability: "toku" | P0 | `[Validation Required]` |

### 14.4 — Architecture Decision Records

| ADR | Decision | Rationale |
|-----|----------|-----------|
| ADR-001 | Core language: Rust | User-confirmed. Best CLI ecosystem, cross-compilation, WASM, FFI. |
| ADR-002 | Database: SQLite + FTS5, normalized schema, Book=Edition with deferred Work grouping | Balance between metadata richness and MVP simplicity. |
| ADR-003 | CLI: clap v4 + subcommands, table/json/csv output, NO_COLOR support | Industry standard. Auto-generates completions and man pages. |
| ADR-004 | Metadata: Open Library primary, Google Books fallback, user edits always win | Free, open, no API key. Fallback for coverage gaps. |
| ADR-005 | Import: streaming with checkpoints, idempotent via source IDs, provenance tracking | Import is the first-impression feature. Must be rock-solid. |

---

## Section 15: Dependency Map

```text
Phase 0: First Book
  ├── toku-core (models, traits)
  ├── toku-db (SQLite, migrations)
  ├── toku-meta (Open Library client)
  └── toku-cli (basic commands)
        │
Phase 1: MVP ──────────── toku-import (Goodreads CSV)
  ├── Reading status       │
  ├── Shelves/tags         │
  ├── FTS5 search          │
  └── Config + completions │
        │                  │
Phase 2: Reading Tracker   │
  ├── Progress logging     │
  ├── Reading sessions     │
  ├── Basic stats          │
  ├── Calibre import ──────┘ (extends toku-import)
  └── Export (CSV/JSON/ZIP)
        │
Phase 3: Analytics & Polish (1.0)
  ├── Full statistics engine
  ├── Mood/pace tags
  ├── Work grouping
  ├── Smart shelves
  └── StoryGraph import
        │
        ├─── Phase 4: Web UI (depends on stable data model)
        │       │
        │       ├─── Phase 5: Native Apps (depends on web + FFI)
        │       │
        │       └─── Phase 6: File Management (independent of web)
        │
        └─── Phase 7: Sync (depends on stable schema)
                │     Self-hosted server, see ADR-010
                └─── Phase 8: Moonshots
```

**Critical path**: `toku-core` → `toku-db` → `toku-cli` → Goodreads import → daily use validation.

**Parallelizable**: Documentation, CI setup, cover image pipeline, due diligence research can all proceed alongside core development.

**FFI timeline**: `toku-ffi` crate work should begin in Phase 3 as a spike to validate Swift interop before Phase 5 depends on it.

---

## Section 16: Feasibility & Compromise Matrix

| Challenge | Ideal Solution | Compromise | Impact | Recommendation |
|-----------|---------------|-----------|--------|---------------|
| Cross-platform core | Single Rust codebase → native + WASM + FFI | CLI-only initially; WASM/FFI validated via spikes | Delays web/mobile but de-risks architecture | **Compromise for MVP, ideal by Phase 5** |
| Goodreads import fidelity | Map every field perfectly | Accept 95% coverage, document gaps | Minor data loss for edge cases | **95% is acceptable** — document missing fields |
| Book deduplication | ISBN + fuzzy title+author + user confirmation | ISBN-only matching in MVP | Misses no-ISBN books | **ISBN-only for MVP** — add fuzzy in Phase 3 |
| Offline metadata | Bundle Open Library subset locally | Require network for first-time fetch; manual entry always works offline | First book add is richer with network | **Network for enrichment, manual entry offline** |
| Stats on large libraries | Pre-computed aggregates, incremental updates | Recompute on demand via SQL | Slow for 10k+ books | **Recompute on demand** — fast enough for <5k books |
| Multi-device sync | CRDT-based conflict-free merge | Manual file copy + last-write-wins | Data loss risk on conflicts | **File copy documented for MVP; CRDTs in Phase 7** |
| Format conversion | Built-in Rust converter | Shell out to Calibre's `ebook-convert` | External dependency | **Shell out** — Calibre is mature and free |

---

## Section 17: Naming & Branding

### 17.1 — Naming Criteria

The name must satisfy:

- ≤6 characters (ideal for CLI: `toku add "Dune"`)
- Pronounceable, memorable, unique on GitHub and crates.io
- Evokes reading, books, or personal knowledge
- Works as `kafkade/<name>` on GitHub
- No existing major project conflicts
- Logo/brand potential

### 17.2 — Name Candidates

| # | Name | Origin/Meaning | CLI Feel | Conflicts | Notes |
|---|------|---------------|----------|-----------|-------|
| 1 | **toku** | Japanese 読く (to read) — phonetic shortening of "doku" (読, reading) | `toku add "Dune"` ✅ | `[Validation Required]` crates.io | **Recommended.** Short, punchy, non-English, memorable. |
| 2 | **lira** | Portuguese/Italian for "to read" (ler/leggere) + evokes lyrical | `lira add "Dune"` ✅ | `[Validation Required]` | Beautiful, international, 4 chars. Runner-up. |
| 3 | **codex** | Latin: "book" (historical — bound manuscript vs scroll) | `codex add "Dune"` ✅ | Multiple projects named Codex `[Validation Required]` | Strong meaning but likely namespace collision. |
| 4 | **folio** | Latin: "leaf" (page of a book), also a book size format | `folio add "Dune"` ✅ | Several projects `[Validation Required]` | Elegant but common. |
| 5 | **tome** | Large or scholarly book | `tome add "Dune"` ✅ | Multiple projects `[Validation Required]` | Evokes heaviness. CLI alias could be `tm`. |
| 6 | **hylde** | Old English: "shelf" | `hylde add "Dune"` ❌ (spelling) | Likely available | Obscure. Spelling is a barrier. |
| 7 | **regal** | German: "shelf" (bookshelf = Bücherregal) | `regal add "Dune"` ✅ | `[Validation Required]` | Nice dual meaning (English: royal). |
| 8 | **stiva** | Romanian: "stack" (of books) | `stiva add "Dune"` ✅ | Likely available | Obscure but musical. |
| 9 | **pila** | Latin: "pile, stack" | `pila add "Dune"` ✅ | `[Validation Required]` | Very short. Could confuse with "pill." |
| 10 | **libro** | Spanish/Italian: "book" | `libro add "Dune"` ✅ | Several projects `[Validation Required]` | Recognizable but probably taken. |
| 11 | **biblio** | Greek: "book" (root of bibliography) | `biblio add "Dune"` ✅ | Likely taken | Too common, longer than ideal. |
| 12 | **verso** | The left-hand page of a book (opposite of recto) | `verso add "Dune"` ✅ | `[Validation Required]` | Typographic, elegant. |
| 13 | **shubi** | Swahili-inspired, phonetic play on "shufu" (to read/look) | `shubi add "Dune"` ✅ | Likely available | Unique, playful. Pronunciation clear. |
| 14 | **tsundoku** | Japanese 積ん読: buying books and never reading them | Too long for CLI | N/A | Great concept but >6 chars. Inspiration for `toku`. |
| 15 | **reki** | Japanese 歴: "history, record" | `reki add "Dune"` ✅ | `[Validation Required]` | Short, punchy, relates to tracking/recording. |

### 17.3 — Recommendation: **Toku**

**Why Toku?**

1. **Meaning**: Derived from Japanese 読 (doku/toku — to read). Directly evokes reading. Also echoes 積ん読 (tsundoku — buying books you don't read), which is deeply relatable for the target audience.
2. **CLI ergonomics**: 4 characters. `toku add "Dune"` is fast to type. No special characters.
3. **Memorability**: Short, distinctive, not a common English word — highly memorable.
4. **Uniqueness**: Less likely to collide with existing projects than `codex`, `folio`, or `libro` `[Validation Required]`.
5. **Brand potential**: Clean, modern feel. Works for logo design (the 読 kanji could be a subtle brand element). Works as `kafkade/toku`.
6. **International**: Non-English origin satisfies the developer's explicit request. Pronounceable in most languages (TOH-koo).

**Runner-ups**:

- **Lira** — beautiful, international, 4 chars. Risk: may collide with cryptocurrency "Lira" or music terms.
- **Verso** — typographic elegance, 5 chars. Risk: niche reference, less immediately evocative of "reading."

### 17.4 — Naming Discussion

**Abstract vs descriptive**: Abstract wins. "BookTracker" is forgettable. "Toku" is distinctive. When mobile apps exist, the abstract name translates seamlessly — "Toku for iOS" works; "BookTracker for iOS" is generic.

**Non-English**: Explicitly requested and strongly recommended. Non-English names create distinctiveness in the English-dominated open-source ecosystem. Japanese origin connects to a literary culture. The name doesn't need to be understood — it needs to be memorable and searchable.

**Product name vs CLI binary**: Keep them the same. `toku` is the binary, the product, the brand. No separate `tk` or `bk` alias needed — 4 characters is already fast enough. Aliases can be added by users who want them.

**Domain**: For an open-source project, `.dev` or GitHub Pages is sufficient. `toku.dev` `[Validation Required]`. A `.com` domain is unnecessary for a CLI tool distributed via GitHub and crates.io.

**The kafkade connection**: `kafkade` is itself a literary reference (Kafka + -ade, or Turkish "kafkade" meaning Kafkaesque). `kafkade/toku` as a GitHub path has a subtle literary resonance — a Kafkaesque reading tool. This is an asset, not a conflict.

---

## Section 17A: Success Metrics & Adoption Signals

### Phase 0 Success

| Metric | Target |
|--------|--------|
| `toku add --isbn` → `toku show` round-trip | Works on 10 different ISBNs |
| CLI startup time | <200ms |
| CI: all platforms green | Linux + macOS + Windows |

### Phase 1 (MVP) Success

| Metric | Target |
|--------|--------|
| Goodreads import: field accuracy | >95% for title/author/rating/date, >80% for all fields |
| Import speed: 300 books from CSV | <5 seconds |
| FTS5 search latency | <100ms on 500 books |
| Developer daily use | Uses `toku` instead of Goodreads for 2+ consecutive weeks |
| Database size | <5MB for 500 books (without covers) |

### Phase 2 Success

| Metric | Target |
|--------|--------|
| Reading progress logged weekly | Developer logs progress ≥1x/week for 4 weeks |
| Calibre import: 1,000 books | Completes in <30 seconds, >90% metadata preserved |
| Export round-trip | backup → import into fresh DB = identical library |

### Phase 3 (1.0) Success

| Metric | Target |
|--------|--------|
| GitHub stars | 100+ (awareness signal) |
| Issues filed by external users | 10+ (engagement signal) |
| First external PR merged | 1+ (contribution model works) |
| Stats engine: all core metrics computed | 12+ statistics types available |

### Quality Metrics (Ongoing)

| Metric | Target |
|--------|--------|
| CLI startup time | <200ms |
| FTS5 search on 10k books | <100ms |
| Zero data loss on import/export round-trip | 100% fidelity |
| No panics in production | Zero unwrap-on-None in user-facing paths |

---

## Section 18: Failure Mode Analysis

| # | Failure Mode | Likelihood | Impact | Mitigation |
|---|-------------|-----------|--------|-----------|
| 1 | **Goodreads CSV format changes** | Low (Amazon neglects Goodreads) | High — MVP import breaks | Store raw CSV alongside parsed data. Version the parser. Monitor format via community reports. |
| 2 | **CLI-only is too niche** | Medium | High — no user growth | Phase 4 (Web) planned early. CLI-first builds the core; web expands the audience. Avoid spending >12 months CLI-only. |
| 3 | **Metadata APIs degrade** | Medium (free tiers shrink) | Medium — enrichment degrades, core works | Multi-source fallback. Manual entry always works. Cache aggressively. |
| 4 | **Feature creep** | High (the prompt itself is ambitious) | High — nothing ships | Strict phase boundaries. 3–5 deliverables per phase. Cut line defined per phase. Ship Phase 1 in <90 days. |
| 5 | **Cross-platform is too ambitious** | Medium | Medium — delays mobile/web | CLI + core library are the product for 1.0. Mobile/web is post-1.0 expansion, not a requirement. |
| 6 | **Data model is wrong** | Medium | High — cascading rewrites | Phase 0 spike validates the model. Book=Edition with deferred Work grouping is conservative. Migration tooling built from day one. |
| 7 | **Solo developer burnout** | High | Fatal | Phase scope is aggressive but realistic. Ship incrementally. Use the app daily — dogfooding sustains motivation. Accept community PRs early. |

---

## Section 19: Decision Log

| # | Decision | Options Considered | Recommendation | Status |
|---|----------|-------------------|----------------|--------|
| 1 | Core language | Rust / Go / TypeScript / Python / Swift | **Rust** — user-confirmed, best CLI ecosystem + FFI + WASM | Decided |
| 2 | Database | SQLite / DuckDB / Flat files | **SQLite + FTS5** — battle-tested, embedded, single-file | Decided |
| 3 | Workspace structure | Monolith-first / Multi-crate workspace | **Multi-crate workspace from day one** — enforces layer boundaries | Decided |
| 4 | CLI framework | clap / structopt / argh | **clap v4 (derive)** — industry standard, auto-completions, man pages | Decided |
| 5 | Metadata source | Open Library / Google Books / ISBNdb / Multi-source | **Open Library primary + Google Books fallback** — both free | Decided |
| 6 | Work vs edition model | Full FRBR / Book=Edition / Deferred | **Book=Edition for MVP, Work grouping Phase 3** — pragmatic | Decided |
| 7 | Rating scale | 5-star / 10-point / 5-star with halves | **0–10 integer (displayed as 5★ with halves)** — Goodreads-compatible | Decided |
| 8 | Sync strategy | File copy / REST server / CRDTs / SQLite replication | **Self-hosted server (Immich-style) with mandatory zero-knowledge E2E encryption and 1Password-style two-secret SRP auth** — see ADR-010 (supersedes ADR-006/008) | Deferred (Phase 7) |
| 9 | License | MIT / Apache 2.0 / GPL / Dual | **MIT** — maximum contributor friendliness | Decided |
| 10 | Web framework | Axum+HTMX / Leptos / SvelteKit / Next.js | **Axum + maud** — server-rendered HTML, inline SVG charts, no client-side framework (see ADR-007) | Decided |
| 11 | Cover storage | Filesystem / Database blobs / Content-addressed | **Filesystem, content-addressed (SHA-256)** — keeps DB small | Decided |
| 12 | Project name | 15 candidates evaluated | **Toku** — short, non-English, evokes reading, terminal-friendly | Decided `[Validation Required]` crates.io |
| 13 | File management | MVP / Post-1.0 / Never | **Deferred to Phase 6 (post-1.0)** — MVP is a tracker, not file manager | Decided |
| 14 | Import priority | Goodreads-first / Calibre-first / Both MVP | **Goodreads first** — user's current tool, MVP killer feature | Decided |
| 15 | iOS approach | Native SwiftUI / React Native / Tauri | **SwiftUI via C FFI** — best UX for Apple platforms | Deferred (Phase 5) |

---

*This roadmap is a living document. Decisions marked `[Validation Required]` should be validated before implementation begins. Phase boundaries are guidelines — ship when the acceptance criteria are met, not when the calendar says so.*

*The single most important milestone: the developer migrates their Goodreads library and uses Toku daily. Everything else follows from that.*
