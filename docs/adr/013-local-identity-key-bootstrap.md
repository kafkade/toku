# ADR-013: Local Identity & Key Bootstrap + First-Sync Upload Semantics

**Status**: Proposed
**Date**: 2026-07-26
**Extends**: ADR-010 (does not supersede it)
**Issue**: #198 (this ADR) · gates #199 (seq-3 implementation) · epic #207

> **Numbering note.** ADR-012 is reserved for the canonical lossless backup & restore format
> (#195, seq 4 of epic #207) and is written separately. This ADR takes 013 per its tracking
> issue (#198).

## Context

ADR-010 established Toku's 1Password-style two-secret model: an account password plus a
device-generated **Secret Key**, mandatory zero-knowledge encryption, and the principle that
**single-device offline use requires no account and no server** — the sync/hosted layer is
entirely opt-in.

That ADR settled *how* auth and encryption work once a user opts in. It did **not** settle three
things that sit on either side of the opt-in boundary. The internal architecture assessment
(§1(1), §4) surfaced them as gaps that make the sync client incomplete end-to-end:

1. **There is no formal local identity bootstrap.** The Secret Key is generated offline
   (`crates/toku-core/src/crypto/secret_key.rs:56`, `SecretKey::generate`, OS CSPRNG), and the
   Emergency Kit can be rendered offline (`toku account emergency-kit`), but these are not
   framed as a first-class *first step* that produces a durable local identity independent of
   any server. Today `signup` is where a Secret Key most naturally comes into being.

2. **First opt-in does not upload pre-existing state.** `signup` and its helper `finalize_device`
   (`crates/toku-sync-client/src/orchestrator.rs:827`, `:750`) persist only credentials, the
   data key, the device row, and config. `push` (`orchestrator.rs:390`) uploads **only**
   `get_unpushed_ops()` — rows already in `sync_ops`. Any book, session, note, or rating created
   *before* opt-in has no op, so it is **silently left unsynced**. The only current path that
   uploads pre-existing state is a manual `toku sync compact` (`crates/toku-cli/src/main.rs:5042`),
   which serializes the snapshot subset (`SnapshotLibrary`: books, book_authors, sessions,
   progress, tags, book_tags, notes, reviews, settings) — **not the full DB** — and is not
   mentioned in the onboarding docs.

3. **New-device bootstrap exists but is not wired or exposed.** `bootstrap`
   (`orchestrator.rs:539`) downloads and applies the latest server snapshot (if any) and then
   pulls remaining ops, but it is **not called by `signup`/`enroll`** and has **no CLI command**.
   A fresh CLI device therefore cannot restore a snapshot; after compaction prunes ops it can
   only pull post-snapshot history.

A fourth item is a documentation hazard rather than a gap: ADR-010's phrase "no local account
before opt-in" is about the **sync identity**. It is easy to read as "no local accounts at all,"
which contradicts `toku-web`, whose durable `web_users` / `web_sessions`
(`crates/toku-web/src/auth.rs`) back the self-hosted dashboard. The split must be stated plainly
so later docs do not appear self-contradictory.

### Relationship to seq-1 (#194)

Seq-1 (#194) adds a reusable local op-emission layer so that every syncable mutation stages a
`SyncOp` atomically, **no-op when no device identity is configured**. That closes the "ongoing
edits after opt-in generate ops" gap, but by design it does **not** emit ops for state that
already existed at the moment of opt-in — a device identity is created *by* opt-in. This ADR
covers exactly that boundary. It is written against the post-#194 architecture; the decisions
here do not depend on #194 being merged, only on op-emission being gated on device identity.

### Constraints that frame every decision

- **Local-first is non-negotiable.** Generating the Secret Key, rendering the Emergency Kit, and
  all core features must work with **no network**. Identity bootstrap is a local act; binding to
  a server is the only networked step and is opt-in.
- **Zero-knowledge is preserved.** Nothing here weakens ADR-010: the server still never sees the
  Secret Key, password, or any plaintext. First-opt-in upload flows through the existing
  per-op AEAD pipeline.
- **User data ownership.** The local SQLite library remains the source of truth and the primary
  recovery path; the server is an encrypted replica.

## Decision

### D1 — Formal local identity bootstrap

The **Secret Key is the durable local identity root**, generated offline and decoupled from
server signup:

- **Generation is a local, offline, first-class step.** A user can create their identity —
  `SecretKey::generate` (OS CSPRNG) — and render the **Emergency Kit exactly once**
  (`toku account emergency-kit`, plain text / printable HTML / PDF) **without any network or
  server**. The formatted, checksummed `TK-…-CC` key and the key material are stored locally
  (OS keychain / token store), zeroized in memory on drop.
- **This local identity is not a server account.** It carries no server relationship. It exists
  purely so the Secret Key and Emergency Kit are durable local artifacts the user controls
  before — and independent of — any opt-in.
- **Binding to a server happens at opt-in, reusing the same Secret Key.** `signup` builds the
  key hierarchy (`AccountKeys::create(password, secret_key)`) from the already-generated Secret
  Key and uploads only the SRP verifier and wrapped keys; `enroll` reuses the Secret Key to
  unwrap the shared library data key. The server sees neither the Secret Key nor the password.
  Both entry points already accept a `SecretKey`, so binding an existing local identity is a
  wiring/UX change, not a protocol change.
- **No server-side escrow** (unchanged from ADR-010). Losing the Secret Key with no local device
  copy leaves server data unrecoverable — this is intentional and documented (`docs/recovery.md`).

The mental model this ratifies: **identity → (optionally) bind to a server → sync**. Identity is
local and offline; the server is an opt-in encrypted replica of a library that already exists.

### D2 — First-opt-in upload: automatic op-backfill

On a successful `signup` — or an `enroll` that creates a **fresh** library from a device that
already holds local data — Toku **automatically backfills the op-log**: it synthesizes
create/set ops for every existing row of the **syncable entity types** and pushes them through
the normal zero-knowledge pipeline. This is automatic and non-interactive; a first-time syncer
never has to know `compact` exists.

Why op-backfill (rather than an auto-snapshot or a prompt-only detector):

- **The op-log stays the single source of truth.** Snapshots remain a pure *compaction*
  optimization of the op-log (as in ADR-008/010), not a parallel state channel. After backfill,
  the server holds a complete op history for the library from op #1.
- **One coverage set, identical to ongoing sync.** Backfill covers exactly the entity types that
  ongoing op-emission covers (Book/Session/Progress/Tag today; Author/Shelf/Work/Series/ISBN via
  #208). There is no second, divergent "snapshot subset" that could drift from the op stream —
  the precise failure #199 flags with `compact` (which serializes a different subset than the op
  path materializes).
- **Reuses the existing pipeline unchanged.** Each backfilled op is encrypted with the library
  data key, AAD-bound to `(entity_type, entity_id, op_type)`, chunked (1000/req), deduplicated
  server-side, and cursor-tracked — no new server endpoint or semantics.
- **Idempotent.** Op IDs are UUID v7 (device-prefixed); re-running signup/backfill is a no-op via
  server dedup and `mark_ops_pushed`. Safe to retry after a partial upload.
- **Non-silent by construction.** Backfill is automatic (nothing is left behind), the CLI reports
  the number of items backfilled and pushed, and it explicitly notes data classes that are **not**
  syncable — ebook file **binaries** (local-only per ADR-011) — so the user's mental model of
  "what reached the server" is correct.
- **History stays small.** After a successful backfill + push, Toku **may** auto-compact into a
  snapshot so a large initial library does not leave thousands of ops in the hot log. The
  authoritative upload is still the backfill; the snapshot is derived from it.

The invariant to hold: **after first opt-in, everything sync covers is on the server, and the CLI
has told the user what sync does not cover.** Nothing is silently unsynced.

### D3 — New-device bootstrap + explicit recovery command surface

- **Bootstrap is wired into enrollment.** After `enroll` mints an **active** device session,
  Toku automatically runs `bootstrap` (download+apply the latest snapshot if present, then pull
  remaining ops) so a fresh device restores the full prior library through the normal CLI path.
  For **approval-pending** devices (no session token until an existing device approves them),
  bootstrap is deferred until the first post-approval `login`, which mints the device session.
- **An explicit `toku sync bootstrap` command** is added as the manual recovery / re-provision
  surface — idempotent, safe to re-run. A `--reset-cursor` flag performs a full re-sync
  (re-download snapshot + re-pull from a fresh cursor) for a device whose local state is
  suspect. This is the "recovery command" the assessment calls for; it complements, and does not
  replace, the automatic on-enroll bootstrap.
- **Recovery hierarchy is unchanged from ADR-010.** The **local SQLite library is the primary
  recovery**; `toku export backup` is the portable escape hatch. `bootstrap` is the
  *server → new-device* restore, valid only while the user holds their Secret Key. Losing the
  Secret Key with no local copy remains unrecoverable by design.

### D4 — Clarify the identity split (sync vs web)

ADR-010's "no local account before opt-in" refers to the **sync identity** only:

- **Sync identity** — the SRP account (verifier), the server-side `user`/`device` records, and
  the account → library data-key hierarchy (ADR-010). This does not exist until opt-in
  (`signup`/`enroll`) and is what "no account before opt-in" means.
- **Web dashboard identity** — the self-hosted web UI has its **own** durable auth plane:
  `web_users` and `web_sessions` (`crates/toku-web/src/auth.rs`), created at web first-run /
  admin onboarding (`create_admin`, `login`, session cookies). It exists independently of the
  sync identity and is not implied or contradicted by "no local account before opt-in."

These are two distinct identity planes today. ADR-013 only **names** the split so downstream docs
are consistent. Whether to unify them (single credential across web + sync) or keep them separate
is deliberately **out of scope here** and deferred to #205 (ADR-015, auth coherence).

## Consequences

Implementation lands in #199 (seq 3, P0). Concretely:

- **toku-cli / toku-sync-client**: `signup` and fresh-library `enroll` invoke op-backfill before
  the first push; report backfilled/pushed counts and the non-syncable-binaries caveat (D2).
- **toku-sync-client**: `enroll` auto-runs `bootstrap` on an active session, deferring for
  pending devices; a new `toku sync bootstrap [--reset-cursor]` command exposes `bootstrap`
  (D3). `finalize_device` remains the identity/config persistence point; backfill is layered
  before push, not inside it.
- **toku-core**: no protocol change. Backfill emits the same `SyncOp`s the op-emission layer
  (#194) already produces; op coverage grows with #208 and backfill coverage grows with it for
  free (same entity set).
- **toku-web**: unchanged; D4 is documentation — the `web_users`/`web_sessions` plane stays as-is.
- **Docs**: `docs/recovery.md` gains the local-identity-first framing and the
  `toku sync bootstrap` recovery verb; onboarding docs stop implying `signup → push → pull` is
  complete without backfill (tracked separately under #202); the identity split (D4) is stated
  wherever "no local account before opt-in" appears.
- **Testing (per #199)**: fresh library → `signup` → books/sessions reach the server with no
  manual `compact`; new device → `enroll` → full prior state restored via bootstrap through the
  real CLI; enrollment-**after-compaction** provisioning passes end-to-end.

Non-goals of this ADR: changing the wire protocol, the key hierarchy, or the zero-knowledge
posture (ADR-010); syncing ebook binaries (ADR-011); unifying web and sync auth (#205); the
backup/restore format (ADR-012 / #195).

## Alternatives Considered

| Option | Rejected because |
|--------|-----------------|
| **First-opt-in upload via auto-snapshot** (upload a `LibrarySnapshot` at signup) | Serializes a *different* subset than the op stream, so new-device correctness would depend on snapshot coverage while incremental correctness depends on op coverage — two divergent code paths that drift (the exact `compact` problem #199 flags). Snapshots also prune op history via `hlc_at_snapshot`, so an incomplete initial snapshot would *permanently* drop detail for entities it does not capture. |
| **Prompt-only** ("we detected N unsynced items — upload?") as the primary mechanism | Reintroduces the silent-unsynced failure the issue exists to eliminate: if the user declines or ignores it, state stays unsynced while the UI implies sync is on. A confirmation prompt is acceptable *UX polish on top of* automatic backfill, not a substitute for it. |
| **Backfill inside `push`** (make `push` also enqueue missing rows) | Overloads `push` with a one-time concern, runs an expensive full-table scan on every push, and blurs the "push = drain the op-log" contract. Backfill belongs at the opt-in boundary. |
| **Generate the Secret Key only at `signup`** (status quo) | Prevents an offline-first identity/Emergency-Kit step and couples a durable user-owned artifact to a networked action. Decoupling (D1) is cheap because `signup`/`enroll` already take a `SecretKey`. |
| **Auto-run bootstrap for pending (approval-required) devices** | A pending device has no session token; bootstrap would fail. Deferring to the first post-approval `login` is the only correct point. |
| **Unify web and sync identities now** | A real product decision with its own trade-offs; owned by #205 (ADR-015). Folding it in here would overreach this ADR's scope. |

## References

- ADR-006 — Sync strategy (superseded by ADR-010)
- ADR-008 — Sync wire protocol (op-log, HLC, snapshots; retained by ADR-010)
- ADR-010 — Self-hosted server, zero-knowledge encryption, two-secret SRP auth, device enrollment
- ADR-011 — File management (ebook **binaries are never synced**)
- ADR-012 / #195 — Canonical lossless backup & restore format (reserved number; separate ADR)
- Issues: #198 (this ADR), #199 (seq-3 implementation), #194 (seq-1 op emission),
  #208 (op-protocol expansion), #205 (ADR-015 web/sync auth coherence); epic #207
- Code: `crates/toku-core/src/crypto/secret_key.rs`,
  `crates/toku-sync-client/src/orchestrator.rs` (`signup`, `finalize_device`, `push`,
  `bootstrap`), `crates/toku-web/src/auth.rs`
- Docs: `docs/recovery.md`, `docs/sync-server.md`, `docs/web-auth.md`
