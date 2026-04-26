# ADR-006: Sync Strategy — Changeset-Based REST with Optional Encryption

**Status**: Accepted (implementation deferred to Phase 7)
**Date**: 2026-04-26
**Decision**: Multi-device sync via a lightweight changeset-based REST API with optional
client-side symmetric encryption. Lighter than Pildora's zero-knowledge model.

## Context

Toku must work fully offline on a single device. Sync is opt-in and additive. Users
want to sync between a desktop CLI, iOS app, and web app. Book data is personal but not
health-critical — it does not require the zero-knowledge architecture used in Pildora.

## Decision

- **Protocol**: Append-only changeset (op-log) synced via REST API.
- **Server**: Thin Axum service storing encrypted or plaintext changesets + sync cursors.
- **Encryption**: Optional client-side symmetric encryption using Argon2id (KDF) +
  AES-256-GCM. User sets a passphrase; server stores opaque blobs.
- **Web app**: Connected companion (server-backed), not full local-first. Native clients
  are authoritative.
- **Deployment**: Self-hosted Docker image first. Managed instance optional and deferred.

### Entity-specific merge rules

| Entity | Strategy |
|--------|---------|
| Books (metadata) | Last-write-wins per field (HLC timestamp) |
| Reading sessions | Append-only (immutable facts, no conflicts) |
| Reading progress | Monotonic (highest page / latest timestamp wins) |
| Notes / Reviews | LWW with conflict detection (store both versions) |
| Deletes | Soft delete with 30-day tombstone retention |
| Cover images | Content-addressed, lazy-fetch (not in critical sync path) |

### What the server can vs. cannot see (with encryption enabled)

| Server CAN see | Server CANNOT see |
|----------------|------------------|
| Op count, timestamps | Book titles, authors, ISBNs |
| Device IDs | Ratings, reviews, notes |
| Entity types (book, session) | Reading progress, shelf names |

### Comparison to Pildora

| Aspect | Pildora | Toku |
|--------|---------|------|
| Encryption | Mandatory E2E, zero-knowledge | Optional symmetric |
| Key hierarchy | Master → Vault → Item | Single derived key |
| Auth | SRP (zero-knowledge) | Standard auth (or passphrase only) |
| Sharing | Encrypted vault sharing | None (personal tool) |
| Recovery | Recovery key + emergency access | Local SQLite is recovery |
| Engineering cost | ~3 months crypto work | ~2–3 weeks sync work |

## Rationale

- Book data is personal but a reading list leak is not a health crisis. Optional
  encryption is the right trade-off.
- A single passphrase → single key avoids the complexity of key hierarchies, SRP, and
  asymmetric wrapping.
- Entity-specific merge rules prevent the "one merge strategy for everything" trap.
- The web app as a connected companion avoids browser-SQLite complexity for a solo dev.

## Alternatives Considered

| Option | Rejected Because |
|--------|-----------------|
| cr-sqlite (CRDTs) | Experimental, unproven on iOS/WASM — kept as research branch |
| File-based sync (iCloud/Dropbox) | SQLite + cloud sync = corruption risk |
| Turso/libSQL | Mandatory cloud dependency conflicts with local-first |
| Full zero-knowledge (Pildora model) | 3x engineering effort for marginal privacy gain |

## Open Questions (to resolve before Phase 7)

- [ ] cr-sqlite maturity — re-evaluate as potential native-client optimization
- [ ] Snapshot compaction strategy for large op logs
- [ ] Device provisioning UX (first-time sync setup flow)
- [ ] Schema migration during sync (how do ops from different app versions coexist?)
