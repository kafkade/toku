# ADR-011: File Management Architecture — `toku-files` Crate, `files` Schema, OPDS Scope

**Status**: Accepted
**Date**: 2026-06-30
**Decision**: Ebook file management lives in a new `toku-files` crate (pure file/disk
logic over `toku-core` models and `toku-db` persistence, no network). Files are tracked
in a dedicated `files` table (one book → many formats), addressed for integrity by
SHA-256. Disk organization uses opt-in path templates. Format conversion is an optional
shell-out to Calibre. OPDS is served local-network-only by default. File **binaries are
never synced**.

## Context

Phase 6 ("Calibre-grade ebook management") adds optional management of the actual ebook
files (`.epub`, `.pdf`, `.mobi`, `.azw3`) on disk. The MVP through Phase 5 is a reading
*tracker*; file management is a distinct product concern that the modular crate design
was always meant to accommodate (ROADMAP §6, §13 Phase 6).

Phase 6 is broken down under the **Phase 6: File Management** milestone (epic #154):
file association (#148), disk organization / templates (#152), integrity & disk usage
(#149), format conversion (#147), and the OPDS server (#150). The foundations are the
`toku-files` crate scaffold and `files` table schema (#153), gated behind this ADR (#151)
per the project's ADR-per-major-decision convention (ADR-001…010).

This ADR opens Phase 6. It ratifies the as-built foundation (the `toku-files` crate and
the `V17__files.sql` migration already exist in the tree) and locks in the design for the
deliverables that follow. Nothing here may contradict the shipped schema or crate.

Two hard constraints frame every decision below:

- **Local-first / Data Boundary Rule** (`.github/copilot-instructions.md`). Ebook files
  are user data. They stay on-device. `toku-files` performs **no** network I/O.
- **Sync boundary** (ADR-006, ADR-010, ROADMAP §6.4 and the Phase 7 cut line). Phase 7
  syncs *metadata* via an encrypted op-log. Ebook **binaries** are explicitly out of
  scope for sync.

## Decision

### 1. Crate boundary — `toku-files`

A new crate `toku-files` owns all file/disk logic:

- **Depends on** `toku-core` (domain models) and `toku-db` (SQLite persistence) only —
  plus leaf utilities (`sha2`, `uuid`, `chrono`, `serde`, `thiserror`, `rusqlite`).
- **No network access.** File management never touches Open Library, Google Books, or the
  sync server. This is enforced by dependency hygiene (no HTTP client in the crate).
- **Pure file/disk logic.** Reads/writes files on the local filesystem, computes
  checksums, and records associations through `toku-db`. It does not own domain rules that
  belong in `toku-core`.
- Errors use `thiserror` (library-crate convention); the CLI wraps them with `anyhow`.

This mirrors ROADMAP §6.4: "File management lives in a new `toku-files` crate that depends
on `toku-core` and `toku-db`… This does NOT affect the metadata sync strategy."

### 2. Schema — the `files` table (`V17__files.sql`)

File associations are stored in a dedicated `files` table (via `refinery`, idempotent),
because a single book routinely has **multiple formats** (e.g. `.epub` + `.pdf`). A
one-to-many table is the correct shape; per-format nullable columns on `books` would not
scale and would not support arbitrary format sets.

```sql
CREATE TABLE files (
    id TEXT PRIMARY KEY NOT NULL,
    book_id TEXT NOT NULL REFERENCES books(id),
    path TEXT NOT NULL,
    format TEXT NOT NULL,              -- epub, pdf, mobi, azw3
    size_bytes INTEGER NOT NULL,
    checksum TEXT NOT NULL,            -- SHA-256, hex-encoded
    created_at TEXT NOT NULL,          -- ISO 8601
    updated_at TEXT NOT NULL           -- ISO 8601
);
CREATE INDEX idx_files_book ON files(book_id);
CREATE INDEX idx_files_checksum ON files(checksum);
```

- `id` is a UUID (v7, time-ordered) stored as text, consistent with other entity IDs.
- `book_id` foreign-keys the existing `books` row (Book = Edition, ADR-002). No new
  columns are added to `books`; the association is fully expressed by this table. (A
  future "primary file" pointer, if ever needed, would be an additive nullable column.)
- `path` records where the file lives on disk (absolute, or relative to the managed
  library root — see §4).
- `format` is a lowercase string constrained by the application to the supported set
  (`epub`, `pdf`, `mobi`, `azw3`).
- `size_bytes` powers disk-usage reporting without stat-ing every file.
- `checksum` is the SHA-256 of the file contents, hex-encoded (see §3).
- `created_at` / `updated_at` are ISO 8601 timestamps.
- Indexes on `book_id` (list a book's files) and `checksum` (integrity checks, dedup).

### 3. File identity & integrity — SHA-256

- **Content integrity is verified with SHA-256** (hex-encoded), the same content-address
  scheme already used for cover images (ADR-002; `toku-meta` stores covers
  content-addressed by SHA-256). Checksums are computed with a streaming reader so large
  files do not need to be buffered in memory.
- **Deduplication** is scoped per book by `(book_id, checksum)`: associating a file whose
  checksum already matches one linked to the same book is rejected as a duplicate rather
  than creating a redundant row.
- **Integrity checking** (#149) recomputes a file's SHA-256 and compares it to the stored
  `checksum` to detect corruption or out-of-band modification.
- **Disk-usage reporting** (#149) aggregates `size_bytes`, grouped by book / format /
  library, with no filesystem walk required for the summary.

### 4. Disk organization — opt-in path templates

- Files may be **referenced in place** (default, non-destructive) or **organized** into a
  managed library directory. The default never moves or renames the user's files.
- When organization is enabled, on-disk layout is driven by a **configurable path
  template**, e.g. `{author}/{title}.{format}` (#152). Placeholders resolve from the
  book's `toku-core` metadata; the format determines the extension.
- The managed library root lives under the Toku data directory (resolved via
  `TOKU_DATA_DIR` or `directories` `ProjectDirs("com", "kafkade", "toku")`), alongside the
  content-addressed cover store — keeping a single portable, user-owned data directory.
- Template resolution sanitizes path segments (filesystem-illegal characters, length
  limits) and resolves collisions deterministically so re-runs are idempotent.
- **Interface** (#152): `toku file organize [<book>|--all]` builds a plan and applies it,
  moving files by default (`--copy` to copy) and updating stored DB paths in a single
  transaction. `--dry-run` previews the plan without touching disk or DB. Configuration
  lives under `[files]` in `config.toml`: `library_root` (defaults to `<data_dir>/library`)
  and `organize_template` (defaults to `{author}/{title}.{format}`). Supported tokens:
  `{author}`, `{title}`, `{series}`, `{format}`, `{year}`.

### 5. Format conversion — optional Calibre shell-out

- `toku convert` shells out to Calibre's `ebook-convert` CLI (#147). Building a converter
  in Rust is a multi-month effort; Calibre's is mature, free, and battle-tested.
- The dependency is **optional and never hard**: the command checks for `ebook-convert`
  in `$PATH` and, if missing, prints installation guidance instead of failing the build or
  blocking any other feature. Toku itself has no Calibre build/link dependency.
- **DRM-free files only.** Toku does not strip DRM and will not add DRM-removal tooling.

### 6. OPDS server — local-network-only by default

- The library can be served as an **OPDS catalog** for e-readers (KOReader, Moon+ Reader,
  etc.) (#150).
- **Local-network-only by default.** The server binds so it is not exposed to the public
  internet out of the box; exposing it beyond the LAN is an explicit user choice.
- **Optional basic authentication** guards the catalog when enabled.
- Consistent with local-first: OPDS serves the user's own device/library; it is not a
  cloud service and adds no external dependency.

### 7. Sync exclusion — binaries never leave the device

- Ebook file **binaries are explicitly excluded from Phase 7 sync** (ROADMAP §6.4 and the
  Phase 7 cut line: "File sync for ebooks (Phase 6 files are not synced in Phase 7)").
- The op-log sync design (ADR-010) relays encrypted *metadata* ops. Even if `files`-table
  *metadata* rows were to participate in sync in a future phase, the file **contents**
  never traverse the network. Per the Data Boundary Rule, ebook binaries are local-only
  user data.

## Rationale

- A dedicated `files` table is the only shape that cleanly supports multiple formats per
  book and per-format integrity/usage data; nullable columns on `books` do not.
- Reusing SHA-256 for file integrity keeps one content-address scheme across covers and
  ebooks and lets integrity checks and dedup share the same primitive.
- Reference-in-place by default respects user data ownership — Toku never silently moves a
  user's files; organization is an explicit, reversible opt-in.
- Optional Calibre conversion delivers broad format support for free without taking on a
  hard dependency or the maintenance burden of a native converter.
- A LAN-only, optionally-authenticated OPDS server extends the library to e-readers while
  preserving the local-first guarantee.
- Excluding binaries from sync keeps Phase 7 tractable (op-log is small, encrypted
  metadata) and avoids the hard problems of large-blob replication and conflict handling.

## Alternatives Considered

| Option | Rejected Because |
|--------|-----------------|
| Per-format nullable path columns on `books` | Caps formats, wastes columns, can't express arbitrary/duplicate formats; no place for per-file checksum/size |
| Store ebook binaries inside SQLite (BLOBs) | Bloats the DB, breaks the portable single-file model, slows backups (same reasoning that keeps covers on disk, ADR-002) |
| Content-address ebooks like covers (path = hash) | Loses the user's meaningful on-disk names/layout; power users want `{author}/{title}` folders, not opaque hashes. Hash is used for integrity, not naming |
| Always move files into a managed library | Destructive; violates data ownership. Reference-in-place is the safe default, organization is opt-in |
| Bundle a Rust ebook converter | Multi-month effort, endless format edge cases; Calibre already solves this for free |
| Add DRM stripping | Legal/ethical scope creep; Toku supports DRM-free files only |
| Expose OPDS publicly by default | Unsafe default for a personal library; LAN-only + optional auth is the local-first choice |
| Sync ebook binaries in Phase 7 | Large-blob replication + conflict resolution is a separate, harder problem; out of scope by the Phase 7 cut line |

## Consequences

- Phase 6 implementation proceeds on the ratified foundation: `crates/toku-files/`
  (crate scaffold + file association) and `V17__files.sql` (`files` table) already exist
  and are consistent with this ADR.
- The remaining deliverables build on this: templates (#152), integrity & usage (#149),
  conversion (#147), OPDS (#150), with file association (#148) as the entry point.
- `toku-files` must stay network-free; a future need for enrichment belongs in `toku-meta`,
  not here. Dependency review should keep any HTTP client out of the crate.
- Users who enable disk organization gain a Calibre-style layout; users who don't keep
  their existing files untouched. Both paths must remain first-class.
- Sync (Phase 7) treats ebook binaries as permanently local. If binary sync is ever
  revisited, it requires its own ADR and a large-blob transport design — it is not implied
  by this decision.

## References

- ROADMAP §6 (Ebook File Management), §6.4 (Architecture Note), §13 Phase 6 & Phase 7 cut
  line
- Data Boundary Rule — `.github/copilot-instructions.md`
- ADR-002 — Database schema, Book = Edition, content-addressed covers
- ADR-006 / ADR-010 — Sync strategy and self-host auth (metadata-only sync boundary)
- Issues: #151 (this ADR), #153 (crate + schema), #148, #152, #149, #147, #150; epic #154
