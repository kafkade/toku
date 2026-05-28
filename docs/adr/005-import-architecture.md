# ADR-005: Import Architecture — Streaming, Idempotent, with Provenance

**Status**: Accepted
**Date**: 2026-04-26
**Decision**: Import is a first-class feature with dry-run, idempotent re-import,
provenance tracking, match confidence levels, and rollback support.

## Context

Many readers have years of data in Goodreads, Calibre, or StoryGraph. Import is a
first-impression feature: if it fails or loses data, users leave immediately. Import
quality is a competitive differentiator.

## Decision

Every importer implements a common `ImportEngine` trait with these capabilities:

1. **Dry-run mode**: `--dry-run` shows what would happen without writing to the database.
2. **Idempotent re-import**: Each imported record stores a source identifier (e.g.,
   `goodreads_id`, `calibre_id`). Re-importing the same file updates tags on existing
   entries and imports new ones (respecting user edits).
3. **Match confidence levels**:
   - Exact: ISBN match → auto-merge
   - High: Title + Author exact match → auto-merge with summary
   - Fuzzy: Partial title match → flagged for user review
   - None: New entry created
4. **Partial failure handling**: Progress checkpointed every 100 books. `--resume`
   continues from last checkpoint.
5. **Field mapping report**: Post-import summary of fields imported, matched, and skipped.
6. **Provenance tracking**: Every imported field records `(source, timestamp)`. User
   edits clear provenance and set `user_override`.
7. **Rollback**: `toku import undo <import-id>` removes books added by that import.
   Note: tags applied to pre-existing books during re-import are not rolled back.
8. **Import log**: `import_logs` table records every operation.
9. **Shelf-to-tag conversion**: Goodreads `Bookshelves` column values are imported as
   tags. Non-standard exclusive shelves are also preserved as tags. Standard exclusive
   shelves (`read`, `to-read`, `currently-reading`) map to `ReadingStatus`.

### Import priority

| # | Source | Phase | Rationale |
|---|--------|-------|-----------|
| 1 | Goodreads CSV | MVP (Phase 1) | User's current tool, largest user base |
| 2 | Manual entry + ISBN lookup | MVP (Phase 1) | Always available |
| 3 | Calibre metadata.db | Phase 2 | Power users, large libraries |
| 4 | StoryGraph CSV | Phase 3 | Growing community `[Validation Required]` |
| 5 | LibraryThing | Phase 3+ | Niche but vocal `[Validation Required]` |
| 6 | Generic CSV | Phase 2 | Catch-all for custom formats |
| 7 | BibTeX | Phase 3 | Academic persona |

## Rationale

- Import failures are the #1 reason users abandon a new tool.
- Idempotent re-import lets users keep their Goodreads account as a backup during
  transition without creating duplicates.
- Provenance tracking enables future re-enrichment without overwriting user corrections.
- The `ImportEngine` trait enables community-contributed importers.

## Consequences

- Each importer needs dedicated test fixtures (real export files from each source).
- The match confidence system adds complexity but prevents silent data corruption.
- Goodreads CSV format instability is a risk — mitigate by storing raw CSV alongside
  parsed data and versioning the parser.
