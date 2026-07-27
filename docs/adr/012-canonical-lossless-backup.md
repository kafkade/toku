# ADR-012: Canonical Lossless Backup & Restore Format

**Status**: Proposed
**Date**: 2026-07-27
**Issue**: #195 (this ADR) · gates #200 (seq-5 implementation) · epic #207
**Supersedes**: the ad-hoc `toku export backup` format (`LibraryExport`)

> **Numbering note.** ADR-012 is the reserved slot for the canonical lossless backup & restore
> format (#195, seq 4 of epic #207). ADR-013 (local identity & key bootstrap, #198) was written
> and merged first under its own tracking issue; the two are independent.

## Context

The ROADMAP and the "user data ownership" non-negotiable promise a **canonical, lossless**
backup in portable open formats with full export and no lock-in. Today that promise is unmet:
Toku ships **two divergent lossy serializers** and **no importer for either**, so the canonical
artifact cannot round-trip and the #185 "100% import/export round-trip fidelity" gate fails.

### The two serializers as-built

1. **Backup export** — `build_library_export` produces a flat `LibraryExport`
   (`crates/toku-export/src/lib.rs:57`), zipped by `export_backup` into `manifest.json` +
   `library.json` + `covers/<hash>.jpg` (`crates/toku-export/src/backup.rs:12`). It is a
   per-book flattening intended for portability.

2. **Sync snapshot** — `SnapshotRepository::export_snapshot` produces a `LibrarySnapshot`
   wrapping a `SnapshotLibrary` (`crates/toku-db/src/snapshot.rs:24`; type at
   `crates/toku-core/src/sync.rs:507`). It exists to compact the op-log and bootstrap new
   devices (ADR-008/010), and is closer to the raw schema — but it is not a portable backup and,
   by design, carries no binaries.

There is **no `toku import backup`**: `ImportSource` offers only Goodreads and Calibre
(`crates/toku-cli/src/main.rs:321`). `apply_snapshot` can ingest a *snapshot*, but only with
`INSERT OR IGNORE` (first-write-wins on primary key), which is a bootstrap path, not a
merge/restore path.

### Exactly what each serializer loses

Precise accounting (this is the "what lossless must add" list #200 will build against):

**`build_library_export` (backup) drops:** reading sessions, reading progress, notes, reviews,
metadata provenance, works (and `work_id`), series / `book_series` (and series position),
`duration_minutes` (audiobook length), user settings, ebook files (associations **and**
binaries), author `id` / `sort_name` / `position` (it keeps only name + role), `tag_type` (it
flattens tags to bare names, collapsing e.g. a "dark" mood tag and a "dark" genre tag),
shelf `id` / `is_smart` / `smart_filter` (smart-shelf definitions), the source IDs
`goodreads_id` and `calibre_id` (making idempotent re-import impossible), any ISBNs beyond a
single ISBN-13 and single ISBN-10 (`lib.rs:85` takes the first of each), and soft-delete
tombstones (`deleted_at` / `deleted_by_device`).

**`SnapshotLibrary` (sync snapshot) drops, relative to the full model:** `isbns`,
series / `book_series`, the `works` table, shelves / `book_shelves`, `metadata_provenance`,
`files`, the source IDs, and `import_logs` — plus, intentionally, **all binaries** (covers and
ebook files are local-only per ADR-011).

The union of what a lossless format must add over *both* is therefore **the whole domain model
plus binaries**.

### Constraints that frame every decision

- **Local-first / offline.** Backup **and** restore must work fully offline with no network and
  no account. Encryption, if used, relies only on a locally held key.
- **User data ownership.** This ADR is central to the ownership promise: portable open formats,
  a complete export, and a restore path with no lock-in.
- **No social features.** The artifact describes one user's library; there is no sharing model.
- **Frictionless, idempotent, lossless import.** Restore re-uses the import philosophy of
  ADR-005: dedup on stable/source IDs, never clobber user edits, safe to re-run.

### Two places where the code contradicts issue #195's assumptions

1. **Identifiers.** #195 lists "ISBN-10/13, ASIN, OpenLibrary ID, Goodreads ID" as first-class
   identifiers. In the shipped schema only **ISBNs** (`isbns` table), **`goodreads_id`** and
   **`calibre_id`** (columns on `books`) are *persisted*. ASIN and Open Library ID appear only
   transiently in the metadata-fetch types (`crates/toku-meta/src/openlibrary.rs:23,36`) and are
   never written to the database. A lossless backup can therefore only be lossless **with
   respect to persisted state**; persisting ASIN / OL-ID is a schema change and is out of scope
   here (a candidate for #200 or a later ADR, not a backup-format concern).
2. **Snapshot restore is not a merge.** `apply_snapshot` uses `INSERT OR IGNORE` throughout
   (`crates/toku-db/src/snapshot.rs`), i.e. no source-ID dedup and no provenance precedence.
   That is correct for empty-DB bootstrap but insufficient for `import backup` into a populated
   library; the restore semantics below (D3) are new work for #200.

## Decision

Toku adopts **one canonical, versioned, lossless backup format** and a matching
`toku import backup` restore. Both are defined by a **single shared domain schema** that also
underpins the sync snapshot's non-binary subset. Five decisions:

### D1 — One shared, versioned domain schema covering the whole model, plus a backup-only binary section

A canonical set of versioned `serde` types lives in **`toku-core`** and covers **every**
persisted entity — not a per-book flattening:

- **Books** — all columns, including `subtitle`, `description`, `page_count`, `pub_date`,
  `language`, `format`, `duration_minutes`, `cover_hash`, `work_id`, `status`, `rating`,
  timestamps, tombstones (`deleted_at`, `deleted_by_device`), and the source IDs `goodreads_id`
  / `calibre_id`.
- **Contributors** — `authors` (`id`, `name`, `sort_name`) and `book_authors`
  (`role`, `position`), preserving the full role set (author / editor / translator /
  illustrator / narrator) and ordering.
- **Identifiers** — the complete `isbns` set per book (all ISBN-10 / ISBN-13, not a single one
  of each).
- **Works** — the `works` table and each edition's `work_id` link.
- **Series** — `series` and `book_series` (including `position`).
- **Shelves** — `shelves` (including `is_smart` / `smart_filter`) and `book_shelves`.
- **Tags** — `tags` (including `tag_type`) and `book_tags`.
- **Reading history** — `reading_sessions` and `reading_progress` in full.
- **Notes / reviews** — full content plus tombstones.
- **Settings** — `user_settings`.
- **Provenance** — `metadata_provenance` per field (`source`, `source_date`, `is_user_override`,
  `sync_hlc`), so per-field source and user-override state survive a round trip.
- **Files** — `files` association rows (path, format, size, checksum, source).

Plus a **backup-only binary section** the snapshot never carries:

- **Cover images**, content-addressed by SHA-256 (`covers/<hash>.jpg`), consistent with
  ADR-002 / ADR-011 content addressing.
- **Ebook files**, content-addressed by the stored `files.checksum` (`files/<checksum>.<ext>`),
  referenced from the `files` entity rows.

The artifact is a **versioned ZIP**: `manifest.json` (`format_version`, `created_at`, entity
counts, `encrypted` flag), `library.json` (the full domain schema above), and the two
content-addressed binary directories. ZIP + JSON is portable, human-inspectable, and free of
SQLite engine/schema-version coupling; it extends the container `export_backup` already writes
rather than inventing a new one.

### D2 — Reuse the schema for the sync snapshot's non-binary subset — shared schema, not identical artifact

The sync snapshot becomes a **projection** of the same versioned `toku-core` types: the
non-binary subset selected for op-log compaction and new-device bootstrap. It is a *shared
schema*, deliberately **not** an identical artifact:

- **Backup includes, over today's snapshot:** ISBNs, series, works, shelves, provenance, file
  associations, source IDs — and the entire binary section (covers + ebook files).
- **Snapshot includes, over today's backup:** the raw session/progress/notes/reviews/settings
  rows the flat export drops (the snapshot is already closer to the schema here).
- **Snapshot excludes, permanently:** all binaries. Ebook and cover **contents** never traverse
  the network (ADR-011); the snapshot stays a compacted op-log citizen.

This retires both current lossy serializers in favour of one schema with one version. The two
consumers differ only by *which tables they project* and *whether binaries are attached*, so
they cannot silently drift the way `LibraryExport` and `SnapshotLibrary` have.

### D3 — Restore / `import backup` semantics: idempotent, merge-by-default, precedence-respecting

`toku import backup <file>` restores into the local database with these rules:

- **Idempotent dedup.** Match an incoming entity to an existing row by **stable `id` (UUID)
  first**, then by **source identifiers** (`goodreads_id`, `calibre_id`, ISBN) for
  cross-instance restores where UUIDs differ. Re-importing the same backup is a no-op; this
  mirrors the importer dedup already used for Goodreads (`goodreads_id`,
  `crates/toku-import/src/goodreads.rs:151`).
- **User-edit & provenance precedence.** A restored field **never clobbers a newer local user
  edit.** Reuse the existing provenance model: per-field `metadata_provenance` with
  `is_user_override` and `sync_hlc` last-write-wins, the same rule the merge engine
  (`crates/toku-db/src/merge.rs`) and local op emission (`crates/toku-db/src/sync_repo.rs`)
  already apply. A field from the backup is applied only if it is newer by HLC and does not
  overwrite a newer user override.
- **Per-entity strategy** (consistent with ADR-006's merge table):
  - Append-only facts (`reading_sessions`, `reading_progress`) — insert-if-absent by `id`;
    never mutated.
  - Notes / reviews — LWW by HLC, honouring tombstones (`deleted_at`); a tombstone in either
    side wins per retention rules.
  - Join rows (authors, tags, shelves, series, ISBNs) — insert-if-absent; membership is additive.
  - Binaries — written content-addressed; an existing hash/checksum match is a no-op (dedup by
    content, per ADR-011).
- **Merge vs replace.** The default is **merge** (additive, precedence-respecting) into a
  possibly non-empty library. An explicit **`--replace`** performs a verbatim restore into a
  cleared/fresh library for disaster recovery. The #185 / #200 round-trip fidelity test uses the
  fresh-DB restore path so that export → import → structural-equality holds exactly.

### D4 — Optional encryption is a wrapper, not a separate schema

An encrypted backup is the **same artifact, sealed** — never a second format:

- The plaintext `library.json` (and, at rest, the binary payloads) are sealed with the
  **library data key** (`SyncKey`) using the existing snapshot AEAD path
  `encrypt_snapshot` / `decrypt_snapshot` (`crates/toku-core/src/crypto.rs:291,325`,
  AES-256-GCM), the same zero-knowledge key hierarchy as sync (ADR-010: account keys →
  wrapped library data key).
- `manifest.json` carries an `encrypted` flag and the `EncryptedEnvelope`; a decryptor with the
  data key recovers the identical plaintext schema. The unencrypted and encrypted variants share
  one schema and one version — encryption is an outer wrapper only.
- **Offline-first holds both ways.** With no key configured, a plaintext backup is produced and
  restored entirely offline (today's behaviour). Encryption is opt-in and local; it requires no
  server.
- This is the natural home for the at-rest / backup-encryption work in **seq-10 (#204)**; the
  format is designed so #204 adds a wrapper, not a new serializer.

### D5 — Versioning and forward/backward compatibility

- A single integer **`format_version`** in `manifest.json`, shared with the snapshot schema so
  the two evolve together. It **starts at 2** (the current flat export is version `"1"`; the
  lossless format is the next generation).
- **Additive changes** (new optional fields or whole new entity tables) do **not** bump the
  major version.
- **Forward-compatible restore:** unknown fields/entities are ignored with a warning rather than
  rejected, so an older Toku can still restore a newer backup's shared subset.
- **Backward-compatible restore:** missing optional fields fall back to schema defaults, so a
  newer Toku restores an older backup.
- A restore **refuses** a `format_version` whose major exceeds what it supports, with a clear
  error, rather than silently dropping data.

## Consequences

Implementation lands in **#200** (seq-5, P0 1.0 blocker), not this ADR. Concretely:

- **toku-core** — defines the canonical versioned schema types (D1) and the `format_version`
  constant (D5); no protocol change to crypto (D4 reuses `encrypt_snapshot`/`decrypt_snapshot`).
- **toku-db** — a full-domain serializer/deserializer replaces the snapshot's partial coverage;
  `apply_snapshot`'s `INSERT OR IGNORE` is superseded by the merge/precedence restore (D3). The
  snapshot becomes the non-binary projection of the shared schema (D2).
- **toku-export** — `export_backup` emits the versioned lossless ZIP with the binary section;
  `build_library_export`/`LibraryExport` is superseded (the flat CSV/JSON/Markdown exports remain
  as separate human-facing reports, unaffected).
- **toku-cli** — adds a `Backup` variant to `ImportSource` (`toku import backup`,
  `crates/toku-cli/src/main.rs:321`) with `--replace` and dry-run, matching the existing importer
  UX; emits an encrypted backup when a key is available (D4, wired fully by #204).
- **Testing (#200 / #185)** — a round-trip fidelity test (export → fresh-DB import → structural
  equality across books, contributors, sessions, progress, notes, reviews, tags, shelves, works,
  series, ISBNs, provenance, files) runs in CI, making the "canonical, lossless" claim true.

**Non-goals of this ADR:** persisting ASIN / Open Library IDs (schema change, out of scope);
syncing binaries (ADR-011 keeps them local); the encryption/key mechanics themselves (ADR-010);
and any change to the op-log wire protocol (ADR-008/010).

## Alternatives Considered

| Option | Rejected because |
|--------|-----------------|
| **Extend the existing `LibraryExport` flat per-book format** | Its shape cannot represent sessions, progress, provenance, works, series, or multi-format files without effectively becoming a new schema; better to define the shared schema once. |
| **Make backup and snapshot the *same* artifact** | Sync intentionally excludes binaries and stays a compacted op-log citizen (ADR-008/010/011). Sharing *types + version* with a table/binary **projection** gives one source of truth without forcing binaries onto the wire. |
| **SQLite `.dump` / `VACUUM INTO` as the backup** | Couples the artifact to the SQLite engine and the exact migration version, is not human-inspectable, and complicates selective/merge restore. JSON-in-ZIP is portable and diff-friendly. |
| **Restore = replace only** | Destroys a populated library and cannot merge a second device's history; violates the frictionless-import and data-ownership goals. Merge is the default; `--replace` is the explicit disaster-recovery escape hatch. |
| **Restore = merge with plain `INSERT OR IGNORE`** (reuse `apply_snapshot`) | First-write-wins ignores source-ID dedup and provenance, so re-import duplicates and can clobber newer user edits — exactly the failures D3 exists to prevent. |
| **A separate encrypted backup schema** | Doubles the formats and risks drift. Encryption as an outer wrapper over the one schema (D4) keeps a single lossless definition and reuses the audited AEAD path. |
| **No `format_version` / rely on ad-hoc detection** | Makes forward/backward compatibility guesswork; an explicit shared version with additive rules (D5) is required for a durable, user-owned format. |

## References

- ROADMAP §3.1.2 (canonical lossless backup), §5 / §5.4 (assessment source)
- Data Boundary Rule — `.github/copilot-instructions.md`
- ADR-002 — Database schema, Book = Edition, content-addressed covers
- ADR-005 — Import architecture (dedup on source IDs, idempotency, provenance precedence)
- ADR-006 — Sync strategy (entity-specific merge rules)
- ADR-008 — Sync wire protocol (op envelope, HLC, snapshot format, encryption envelope)
- ADR-010 — Self-hosted server, zero-knowledge key hierarchy, library data key
- ADR-011 — File management (`files` table, SHA-256 content addressing, binaries never synced)
- ADR-013 — Local identity & key bootstrap (snapshot subset context)
- Issues: #195 (this ADR), #200 (seq-5 implementation), #185 (round-trip fidelity gate),
  #204 (seq-10 at-rest / backup encryption); epic #207
- Code: `crates/toku-export/src/lib.rs:57`, `crates/toku-export/src/backup.rs:12`,
  `crates/toku-db/src/snapshot.rs`, `crates/toku-core/src/sync.rs:507`,
  `crates/toku-cli/src/main.rs:321`, `crates/toku-core/src/crypto.rs:291`,
  `crates/toku-db/src/merge.rs`, `crates/toku-import/src/goodreads.rs:151`,
  `crates/toku-meta/src/openlibrary.rs:23`
