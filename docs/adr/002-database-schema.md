# ADR-002: Database — SQLite with FTS5, Book=Edition Model

**Status**: Accepted
**Date**: 2026-04-26
**Decision**: Use SQLite with FTS5 for the local database. Start with a flat
Book=Edition model; add Work grouping in Phase 3.

## Context

Toku needs an embedded, cross-platform database that supports full-text search, works
offline, and stores all library data in a single portable directory. The book domain has
a well-known modeling challenge: a "book" can mean a work (abstract), an edition
(specific publication), or a copy (the user's item).

## Decision

- **SQLite 3** via `rusqlite` as the primary database.
- **FTS5** virtual table for full-text search across titles, authors, descriptions,
  notes, and reviews.
- **Normalized schema** with join tables for book-author, book-series, book-shelf, and
  book-tag relationships.
- **Book = Edition** for MVP: each row in the `books` table represents a specific edition.
  A nullable `work_id` column is reserved from day one but not populated until Phase 3.
- **Schema migrations** via the `refinery` crate, embedded in the binary.

## Rationale

- Full FRBR (Work → Expression → Manifestation → Item) is overengineered for a personal
  library of 100–500 books. Most users think in terms of editions, not works.
- The deferred `work_id` column allows grouping editions later without a schema migration.
- SQLite is battle-tested, single-file, cross-platform, and embeddable in WASM.
- FTS5 provides sub-100ms full-text search on 10k+ books.

## Alternatives Considered

| Option | Rejected Because |
|--------|-----------------|
| DuckDB | Analytical focus, not designed for transactional CRUD |
| Flat files (JSON/TOML) | No full-text search, poor query performance at scale |
| Full FRBR from day one | Over-engineered for MVP, slows iteration |

## Consequences

- Migration from flat Book=Edition to Work grouping in Phase 3 requires careful schema
  evolution. The `refinery` migration system handles this.
- Cover images stored on filesystem (content-addressed by SHA-256), not in SQLite, to
  keep the database file small and portable.
