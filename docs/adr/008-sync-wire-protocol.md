# ADR-008: Sync Wire Protocol — Op-Log Format, HLC, and Encryption Envelope

**Status**: Proposed (implementation in Phase 7)
**Date**: 2026-05-31
**Decision**: Define the sync wire protocol using UUID v7 op IDs, Hybrid Logical Clocks
for ordering, a versioned JSON op envelope, and an optional AES-256-GCM encryption layer.

## Context

ADR-006 established the sync architecture: changeset-based REST with optional symmetric
encryption. This ADR specifies the wire-level details that ADR-006 left open: op format,
clock semantics, cursor contracts, encryption envelope, and versioning strategy.

These decisions must be made before implementation because they affect the local database
schema (`sync_ops` table), the server storage format, and the client-server API contract.
Changing the wire format after devices are syncing requires a migration path.

## Decision

### Op envelope format

Every mutation produces an op. The canonical JSON representation:

```json
{
  "v": 1,
  "op_id": "019726a4-...",
  "device_id": "019726a3-...",
  "hlc": "2026-06-15T10:30:00.000Z-0001-019726a3",
  "entity_type": "book",
  "entity_id": "01972123-...",
  "op_type": "update",
  "fields": {"rating": 8, "status": "read"},
  "checksum": "sha256:abcdef..."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `v` | integer | Envelope version. Currently `1`. Clients reject ops with `v` > their max supported. |
| `op_id` | UUID v7 | Globally unique, time-sortable op identifier. |
| `device_id` | UUID v7 | Originating device. |
| `hlc` | string | Hybrid Logical Clock timestamp (see below). |
| `entity_type` | string | One of: `book`, `session`, `progress`, `tag`, `note`, `review`, `setting`, `device`. |
| `entity_id` | UUID | The entity being modified. |
| `op_type` | string | One of: `create`, `update`, `delete`. |
| `fields` | object | Field-level changes. Only present for `create` and `update`. Keys are field names; values are the new values. |
| `checksum` | string | `sha256:` prefixed hash of the canonical JSON without the checksum field. Detects corruption. |

### Hybrid Logical Clock (HLC)

Format: `{ISO-8601-timestamp}-{counter:04d}-{device_id_prefix:12}`

- Physical clock: system UTC time, clamped to never go backward.
- Logical counter: monotonically increasing per device within the same millisecond.
- Device prefix: first 12 characters of the device UUID for tie-breaking.

HLC comparison: lexicographic string comparison produces correct causal ordering.

**Why HLC over simple timestamps?** Wall clocks drift. Two devices editing the same
field at "the same time" need deterministic ordering. HLC provides causality guarantees
that wall clocks cannot.

### Cursor contract

- Each device maintains two cursors: `push_cursor` (last op ID pushed) and `pull_cursor`
  (last op ID received from server).
- Cursors are op IDs (UUID v7), not timestamps — immune to clock skew.
- Push is idempotent: the server deduplicates by `op_id`. Retrying a failed push is safe.
- Pull is idempotent: requesting the same cursor returns the same ops.
- The server stores one cursor pair per device per library.

### Encryption envelope

When encryption is enabled, the `fields` value is replaced with an encrypted blob:

```json
{
  "v": 1,
  "op_id": "...",
  "device_id": "...",
  "hlc": "...",
  "entity_type": "book",
  "entity_id": "...",
  "op_type": "update",
  "encrypted": {
    "ev": 1,
    "alg": "aes-256-gcm",
    "nonce": "base64...",
    "ciphertext": "base64...",
    "aad": "v=1,entity_type=book,op_type=update"
  },
  "checksum": "sha256:..."
}
```

| Field | Description |
|-------|-------------|
| `ev` | Encryption envelope version. |
| `alg` | Algorithm identifier. Only `aes-256-gcm` for v1. |
| `nonce` | 96-bit random nonce, base64-encoded. Must be unique per op. |
| `ciphertext` | Encrypted `fields` JSON, base64-encoded. |
| `aad` | Additional Authenticated Data — unencrypted metadata bound to the ciphertext. Prevents op type/entity type from being swapped. |

**Nonce uniqueness**: Generated via OS CSPRNG (`getrandom`). UUID v7 op IDs provide
a secondary uniqueness guarantee — if the same nonce were reused (astronomically unlikely),
the different op IDs in the AAD would cause decryption to fail rather than silently
produce wrong data.

**Key derivation**: Passphrase → Argon2id (m=64MB, t=3, p=1) → 256-bit key. Salt is
per-library (generated at `toku sync init`), stored on the server alongside the library ID.

### Schema versioning during sync

- Each op carries `v` (envelope version). The server stores ops as-is.
- Clients must handle all `v` values ≤ their current version.
- A new envelope version means a new op field or structural change. Old fields are
  never removed — only added.
- App-level schema migrations (e.g., new columns on `books`) are separate from sync
  envelope versions. A new database column appears as a new field key in `fields` —
  older clients ignore unknown keys.

### Error semantics

| Error | Client behavior |
|-------|----------------|
| Push conflict (duplicate op_id) | Ignore — op already stored. |
| Pull cursor not found | Full re-sync from latest snapshot. |
| Auth failure (401) | Prompt for device re-authentication. |
| Envelope version too high | Log warning, skip op, prompt user to update app. |
| Checksum mismatch | Reject op, log error, do not apply. |
| Decryption failure | Reject op, log error. May indicate wrong passphrase. |

## Rationale

- **UUID v7 op IDs** are time-sortable and globally unique without coordination —
  ideal for offline-first devices that generate ops independently.
- **HLC** provides causal ordering without requiring synchronized clocks. It's well-studied
  and used by CockroachDB, Jepsen-tested systems, and other distributed databases.
- **Field-level ops** (not row-level) minimize conflict surface. Two devices editing
  different fields of the same book produce two ops that merge without conflict.
- **Versioned envelope** avoids the "big bang migration" problem — new features can be
  added without breaking existing clients.
- **AAD in encryption** binds the entity type and op type to the ciphertext, preventing
  an attacker from swapping encrypted payloads between ops.

## Consequences

- The `sync_ops` table grows linearly with mutations. Snapshot compaction (periodic
  full-state snapshots that allow op-log truncation) is required for long-lived libraries.
- Field-level ops mean the op payload varies by entity type. Clients must validate
  field names against the current schema.
- The encryption envelope adds ~33% size overhead (base64) and CPU cost (AES-GCM per op).
  Acceptable for book metadata; would not scale to high-frequency data.
- Changing the passphrase requires re-encrypting all ops — this is a client-side batch
  operation that can take minutes for large libraries.

## Open Questions

- [ ] Maximum op batch size for push/pull requests (start with 1000 ops per request)
- [ ] Snapshot format (full library JSON vs. compacted op sequence)
- [ ] Rate limiting strategy for the sync server
