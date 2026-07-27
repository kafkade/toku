# ADR-016: Optional At-Rest Database Encryption + Portable Encrypted Backups

**Status**: Proposed
**Date**: 2026-07-27
**Extends**: ADR-010 (zero-knowledge sync), ADR-012 (canonical lossless backup); does not supersede either
**Issue**: #204 (seq 10 of epic #207)

> **Numbering note.** ADR-014 is reserved for the managed multi-tenant SaaS design (#203) and
> ADR-015 for web/sync auth coherence (#205); both are tracked separately. This ADR takes **016**
> per its position in the sequence. Earlier planning notes that guessed "ADR-015" for this work
> are superseded by this note.
>
> **Scope split.** This ADR is the **design gate**. It documents the full at-rest-encryption
> design so a focused implementation PR can follow. Only one piece ships alongside this ADR:
> **portable, passphrase-encrypted backups for offline-only users** (Decision C), which needs no
> new dependency. The heavyweight SQLCipher work (Decisions A/B) is **deferred to a follow-up
> implementation issue** that references this ADR. The follow-up is flagged to the coordinator to
> file; this ADR does not create it.

## Context

Toku is local-first and CLI-first: the entire library — books, reading sessions, notes, ratings,
provenance — lives in a local SQLite database (`toku.db`) that works with no account, no server,
and no network. ADR-010 established that the **optional** hosted-sync layer is zero-knowledge: the
relay stores only client-encrypted ciphertext. ADR-012 established a canonical, lossless,
self-describing backup container (JSON-in-ZIP) with an optional AEAD wrapper over `library.json`.

Two local-confidentiality gaps remain, both called out in #204 and in ADR-010 §A3:

1. **The local `toku.db` is plaintext at rest.** Every open path in
   `crates/toku-db/src/database.rs` (`Database::open`, `open_no_migrate`, `open_in_memory`) sets
   only `journal_mode=WAL` and `foreign_keys=ON`. `rusqlite` links **bundled standard SQLite**
   (root `Cargo.toml`: `rusqlite = { version = "0.39", features = ["bundled", "column_decltype"] }`),
   not SQLCipher. Confidentiality at rest therefore depends entirely on **OS full-disk
   encryption**. Device theft on a machine without FDE exposes the whole library.

2. **Offline-only users cannot encrypt a backup.** `toku export backup --encrypt` exists (#200,
   ADR-012), but the CLI gated it behind an **enrolled sync key** (`load_library_key` requires a
   configured sync server). A user who never opts into sync could not produce a ciphertext backup.

### Constraints that frame every decision

- **Local-first / offline.** Every core feature must work with no network. At-rest encryption must
  not add a network dependency, and the **disabled path must be byte-for-byte identical to today**
  (an unencrypted DB opens exactly as it does now).
- **Opt-in, off by default.** Encryption is a choice, never imposed. A brand-new install is
  unencrypted unless the user asks otherwise.
- **User data ownership / no lock-in.** The user can always migrate an encrypted DB back to
  plaintext and export their data. Keys and formats are documented and open.
- **CLI-first, but shared core.** The same `toku.db` is opened by the CLI, the `toku serve` web
  dashboard, the FFI consumed by the native apps, and the sync client. Any key mechanism must work
  at every entry point.
- **Do not break the build gate.** CI's `apple` job cross-compiles `toku-ffi` for
  `aarch64-apple-ios-sim` and `aarch64-apple-watchos-sim`. SQLCipher + vendored OpenSSL is
  notoriously hard to cross-compile there; the design must keep that build green.

## Decision

### D1 — SQLCipher, feature-gated on `toku-db`, off by default

Add at-rest encryption via **SQLCipher** (`PRAGMA key`), enabled through an **off-by-default cargo
feature** on `toku-db` (working name `sqlcipher`). The feature swaps `libsqlite3-sys`'s bundled
SQLite for a bundled SQLCipher build — the intended selector is
**`bundled-sqlcipher-vendored-openssl`** (vendored OpenSSL avoids a system-library dependency;
exact feature name to be confirmed against `rusqlite 0.39` / `libsqlite3-sys` at implementation
time).

Scoping the feature to `toku-db` — rather than flipping the workspace-level `rusqlite` dependency —
is deliberate:

- The **`toku-sync` relay** has its own `SyncDatabase` and stores only zero-knowledge ciphertext;
  it must **not** be re-keyed with a user's local key. A workspace-wide swap would drag it in.
- The default workspace build, and the CI `test` / `msrv` / `audit` / **`apple`** jobs, stay on
  standard SQLite, so the merge gate is unaffected until the implementation PR deliberately adds a
  dedicated leg that compiles the feature.

When the feature is **off** (the default), `database.rs` is unchanged and there is no SQLCipher
code path at all.

### D2 — Apply the key immediately after open, on all three paths

When the feature is on **and** a key is supplied, each open path must issue `PRAGMA key` (and any
`PRAGMA cipher_*` compatibility pragmas) **as the very first statement after `Connection::open`,
before** `journal_mode=WAL`, `foreign_keys=ON`, and before any migration or query. SQLCipher
requires the key before the database header is read. Concretely, the three constructors gain keyed
variants (e.g. `Database::open_encrypted(path, &key)`, `open_no_migrate_encrypted`,
`open_in_memory` stays plaintext for tests). The existing unkeyed functions remain and behave
exactly as today, preserving the disabled path.

A wrong key is detected on the **first** statement after `PRAGMA key` (SQLCipher returns
`SQLITE_NOTADB` / "file is not a database"). That must surface as a clear
"incorrect passphrase or not an encrypted Toku database" error — never a panic and never a partial
open.

### D3 — Key source: dedicated passphrase → Argon2id → key (B1)

The key is derived from a **dedicated DB passphrase**, independent of sync:

```text
DB passphrase --Argon2id (m=64MB, t=3, p=1, 16-byte salt)--> 256-bit key --PRAGMA key--> toku.db
```

This reuses the existing, audited derivation in
`crates/toku-core/src/crypto.rs::SyncKey::derive` (same Argon2id parameters as sync). Only the
**salt + KDF parameters + a verifier** are persisted (in `config.toml` under a new `[encryption]`
section); the passphrase and the derived key are **never** written to disk. Rationale: an
offline-first, no-account tool must let a user with no sync enrollment encrypt their DB.

**Rejected / secondary alternatives** (see Alternatives Considered): reusing the sync identity
(couples local at-rest to sync enrollment — bad for offline-only users); an OS-keychain-managed
random key as the *sole* mechanism (awkward on headless Linux, ties confidentiality to the login
keychain). A keychain-stored passphrase and a `TOKU_DB_PASSPHRASE` env var are acceptable
**convenience layers on top of** the passphrase, not replacements for it.

### D4 — Key availability across short-lived CLI processes (open question, flagged)

The CLI runs many short-lived processes, so prompting for a passphrase on **every** invocation is
hostile. The implementation PR must choose among (each with a distinct threat trade-off — this is a
**maintainer security decision**, documented here, not resolved now):

- **Interactive prompt** (via `rpassword`) on each unlock — simplest, but painful for scripting and
  repeated commands.
- **OS keychain** (the repo already uses `apple-native-keyring-store`) — best UX, but ties
  confidentiality to the login session and is awkward headless.
- **`TOKU_DB_PASSPHRASE` env var** — automation escape hatch; exposes the passphrase to the process
  environment.
- **Ephemeral unlocked-session token** (a short-lived agent) — good UX, more moving parts.

Recommendation: passphrase prompt as the baseline, with keychain and env var as opt-in
conveniences.

### D5 — Per-entry-point open wiring

Every site that opens `toku.db` must obtain the key and call the keyed constructor when encryption
is enabled:

- **CLI** — the primary dispatch (`crates/toku-cli/src/main.rs`, ~L1044) and the two other opens
  (~L5173, ~L5351). This is where the passphrase is prompted/resolved.
- **Web (`toku serve`)** — the migrating open (`crates/toku-web/src/lib.rs`, ~L227) plus every
  request-time `Database::open_no_migrate` in the handlers (`handlers.rs`, `library_handlers.rs`,
  `opds.rs`, `sync_status.rs`, `auth.rs`, `import_handlers.rs`, `conflicts_handlers.rs`). The
  server is a trusted local process that holds the key for its lifetime.
- **FFI** — `toku_open` (`crates/toku-ffi/src/lib.rs`, ~L204) needs a **key-carrying variant**
  (e.g. `toku_open_encrypted(path, key)`); this is a signature/ABI addition the Swift/iOS/macOS/
  watchOS apps adopt, and the regenerated `toku.h` must ship with it.
- **Sync client** — `crates/toku-sync-client/src/orchestrator.rs` (~L187) opens the same local DB.

### D6 — Migration: plaintext ↔ encrypted, both directions

For data ownership and no-lock-in, the implementation provides an explicit, idempotent migration
(e.g. `toku db encrypt` / `toku db decrypt`) using SQLCipher's `sqlcipher_export()` (attach a new
keyed/plaintext database and copy). The command keeps a backup copy of the original file until the
new one is verified, and refuses to run destructively without confirmation. Re-running when already
in the target state is a no-op.

### D7 — Encrypted backups for offline-only users (ships now)

Acceptance criterion (b) of #204 — an encrypted, restorable backup — is **already met for sync
users** via the enrolled library key (#200). The remaining gap is offline-only users. This ADR
ships that piece because it needs **no new dependency**:

- `toku export backup --encrypt`, when **no sync server is configured**, prompts for a passphrase
  (or reads `TOKU_BACKUP_PASSPHRASE`), derives a key via `SyncKey::derive`, and seals the archive
  with the existing AEAD snapshot envelope.
- **Portability requirement:** the KDF **salt + parameters travel inside `manifest.json`** (a new
  optional `kdf` descriptor on `BackupManifest`). The AEAD envelope carries only the nonce, so
  without the embedded salt a passphrase backup could not be restored on another machine. Embedding
  it makes the archive **self-describing**: restore re-derives the key from the passphrase alone,
  with no dependency on local `config.toml`.
- **Precedence:** if a sync server is configured, `--encrypt` uses the enrolled library key exactly
  as before (no `kdf` in the manifest). Only non-sync users take the passphrase path. On restore,
  the artifact is self-selecting: `manifest.kdf` present ⇒ passphrase backup; absent ⇒ sync-key
  backup.

This keeps the sync path unchanged and gives offline users first-class encrypted backups. It is
independent of the SQLCipher work and does not touch `database.rs`.

### D8 — Threat model

At-rest DB encryption defends against **offline device/disk theft** on a machine without OS
full-disk encryption. It does **not** defend against a compromised running process, a keylogger, or
a weak passphrase, and — like all four of Toku's encryption guarantees — it is distinct from
in-transit (operator TLS), relay zero-knowledge (E2E ciphertext), and the trusted local web
dashboard. `docs/security/self-host-threat-model.md` is updated to retire the "local DB is
plaintext" gap (conditional on opt-in) and to describe portable passphrase backups.

## Consequences

- **Positive:** opt-in defense-in-depth against device theft; offline users get portable encrypted
  backups today; no change to the default experience; sync and relay postures untouched.
- **Cost / risk:** SQLCipher enlarges the encrypted build and adds an OpenSSL cross-compile burden
  — highest for the `apple` (iOS/watchOS) target (**flagged**). Mitigated by feature-gating and
  keeping the feature **off** in default/`apple` CI; the implementation PR adds a dedicated CI leg
  that compiles the feature to prove it builds.
- **Lost passphrase = unrecoverable.** Both an encrypted DB and a passphrase backup are
  unrecoverable if the passphrase is lost — there is no backdoor by design. Documented prominently
  in `docs/recovery.md`.
- **CI / Terraform:** no CI job is renamed under any option, so **no change to
  `kafkade/github-infra`'s `repo_toku.tf` `required_status_checks`** is required. The apple/build
  risk is the item to watch.

## Implementation notes (for the deferred follow-up)

- **Files:** root `Cargo.toml` + `crates/toku-db/Cargo.toml` (feature/dep);
  `crates/toku-db/src/database.rs` (keyed constructors on all three open paths);
  `crates/toku-core/src/config.rs` (`[encryption]` salt/params/verifier);
  CLI opt-in + unlock (`toku-cli`); web open sites; `toku-ffi` (`toku_open_encrypted` +
  regenerated `toku.h`); migration command; docs.
- **Tests:** open-with-key roundtrip; wrong-key clean failure; disabled-path-unchanged;
  plaintext↔encrypted migration idempotency; plus a CI leg that compiles the `sqlcipher` feature.

## Alternatives Considered

| Option | Rejected because |
|--------|-----------------|
| **Full workspace SQLCipher swap now** (flip `rusqlite` for every crate) | Highest risk: likely breaks the `apple` iOS/watchOS OpenSSL cross-compile — the merge gate — and re-keys the `toku-sync` relay DB, which must stay zero-knowledge ciphertext. Enlarges every build for a feature most of them never use. |
| **Reuse the sync identity for the DB key (B2)** | Couples local at-rest encryption to sync enrollment, so an offline-only user could not encrypt their DB — contradicts local-first. The sync Secret Key + password derive a *data* key for the relay, a different purpose. |
| **OS-keychain-managed random key as the sole mechanism (B3)** | Ties confidentiality to the login keychain, is awkward/absent on headless Linux, and offers no user-memorable recovery path. Retained only as an optional convenience layer over the passphrase. |
| **Application-layer field encryption instead of SQLCipher** | Would leave indexes, FTS5 content, and the schema in plaintext, breaking search and defeating the goal; reinvents a well-solved primitive. |
| **Encrypt backups by storing the salt in local `config.toml`** | A backup restored on another machine would not have that config, so it would be unrestorable — violates portability. The salt must travel **in** the archive (D7). |
| **Bump `BACKUP_FORMAT_VERSION` for the new `kdf` field** | Unnecessary: `kdf` is an additive, `#[serde(default)]` optional field, backward/forward compatible per ADR-012 D5. Older Toku ignores it; newer Toku defaults it to absent. |

## References

- ADR-010 — Self-hosted server, zero-knowledge encryption, two-secret SRP auth (§A3 documents the
  current plaintext-local-DB state and names #204 as the follow-up)
- ADR-011 — File management (ebook binaries; cover/file content addressing)
- ADR-012 / #200 — Canonical lossless backup & restore format (the AEAD envelope and manifest this
  ADR extends with a portable `kdf` descriptor)
- ADR-013 — Local identity & key bootstrap (offline Secret Key generation)
- Issues: #204 (this ADR / seq 10), #203 (ADR-014, SaaS), #205 (ADR-015, auth coherence),
  #200 (backup format); epic #207
- Code: `crates/toku-db/src/database.rs`, `crates/toku-core/src/crypto.rs` (`SyncKey::derive`),
  `crates/toku-core/src/backup_schema.rs` (`BackupManifest`, `BackupKdf`),
  `crates/toku-export/src/backup.rs`, `crates/toku-cli/src/main.rs`
- Docs: `docs/security/self-host-threat-model.md`, `docs/recovery.md`, `docs/self-hosting.md`
