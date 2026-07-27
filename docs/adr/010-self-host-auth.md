# ADR-010: Self-Hosted Server with Mandatory Zero-Knowledge Encryption and 1Password-Style SRP Authentication

**Status**: Accepted
**Date**: 2026-06-26
**Supersedes**: ADR-006, ADR-008
**Issue**: #114 (Epic), #115 (this ADR)

## Amendment — 2026-07-27: Implementation Reconciliation (#197)

> This dated amendment reconciles ADR-010 with the shipped implementation as part
> of the on-device-first architecture assessment (epic #207, seq 6). The original
> **Decision** below is preserved as written; where the shipped construction
> differs, this amendment is authoritative and the affected passages carry an
> inline `Amended (#197)` pointer. **No code changed** — the shipped security
> properties are correct for a personal, no-sharing, local-first tool; only this
> ADR's depiction was stale. This amendment extends, and stays consistent with,
> ADR-013.

### A1 — Master unlock key: shipped KDF is a two-step construction

The "Key Hierarchy" diagram and bullets below depict the Secret Key and account
password entering **Argon2id together** to yield the unlock key. The shipped
derivation (`crates/toku-core/src/crypto/key_hierarchy.rs`,
`MasterUnlockKey::derive`) is instead **two steps**, and only the password is run
through the memory-hard KDF:

```text
password_derived = Argon2id(password, salt=per-account, m=64 MB, t=3, p=1) -> 32 bytes
unlock_key       = SHA-256( "toku/master-unlock-key/v1" || Secret Key || password_derived )
```

- The Secret Key is folded in via a **domain-separated SHA-256 combine**, not by
  being fed into Argon2id. Running only the (low-entropy) password through the
  memory-hard KDF while combining the (128-bit, high-entropy) Secret Key with a
  fast hash is the intended design — the Secret Key needs no key-stretching.
- The shipped algorithm identifier is `argon2id+sha256-2skd`
  (`MASTER_KDF_ALGORITHM`).
- Naming: the code type is `MasterUnlockKey`; this ADR calls it the "Account
  Unlock Key". They are the same key.

The security intent is unchanged from the original decision — both secrets are
required to derive the unlock key. Only the construction differs, so this is a
documentation reconciliation, not a code change.

### A2 — Data-key cardinality: one wrapped data key per account (not per library)

**Decision (maintainer-confirmed): the data key is per-account.** The original
"one key per library, generated at library creation" wording is superseded.

Shipped reality:

- `crates/toku-sync/migrations/V7__wrapped_data_key.sql` stores a single
  `wrapped_data_key` column on the **`users`** table — one wrapped data key per
  **account**. `AccountKeys::create` generates exactly one leaf `SyncKey` per
  account; `signup`/`enroll` recover that one account data key and reuse it for
  whichever library the enrolling device lands on.
- The server does support **multiple libraries per user**: `enroll_device`
  (`crates/toku-sync/src/handlers.rs`) mints a fresh library (UUID v7) whenever a
  device enrolls without naming a `library_id`, and `libraries.user_id`
  (migration V5) links many libraries to one account.

**Consequence (stated plainly):** a single account's multiple libraries are **not
cryptographically isolated from each other** — they are all encrypted under the
same per-account data key, so compromise of that data key exposes every library
the account owns. This is an accepted posture for a **personal, single-user tool
with no sharing feature**: there is no second party from whom one library must be
kept secret, so inter-library key separation buys nothing today.

**Rationale:** consistent with this ADR already rejecting per-item keys as
"unnecessary complexity for a personal tool" (see Alternatives Considered); and it
gives the simplest recovery — one key to wrap at signup and unwrap per device
enroll.

**Migration path (if ever needed):** should per-library isolation become a real
requirement (for example a future selective-sharing feature), move to per-library
`wrapped_data_key` rows keyed by `(user_id, library_id)` and generate a distinct
`SyncKey` per library at library creation — matching the original diagram below.
No such work is planned or tracked; this note only records the exit path.

**Naming debt:** the code and this ADR still call the leaf key the "library data
key" (`WrappedDataKey`, `DATA_KEY_WRAP_INFO = "toku/library-data-key-wrap/v1"`,
and the diagram below) even though its scope is per-account. Left as-is to avoid a
churny rename; flagged here so the term is not read as implying per-library scope.

### A3 — "Encrypted at rest" is scoped to the server and in transit, not the local DB

This ADR's zero-knowledge guarantees — "the server stores only encrypted blobs",
mandatory `encrypted` envelopes, and wrapped keys — apply to **data at the server
and in transit**, plus the **zero-knowledge-wrapped data key**. They do **not**
imply that the local working database is encrypted at rest.

- The local `toku.db` is **not encrypted at rest today**: `toku-db`'s database
  open path (`crates/toku-db/src/database.rs`) sets only `journal_mode=WAL` and
  `foreign_keys=ON` — there is no SQLCipher or other at-rest encryption.
- Optional at-rest DB encryption (SQLCipher) plus encrypted backups are separate,
  future work tracked in **#204** (epic #207, seq 10).
- The same overclaim in `.github/copilot-instructions.md`'s data-boundary table
  ("Local SQLite (encrypted at rest if sync enabled)") is corrected under **#196**
  (seq 7); this amendment and #196 must stay consistent.

Local-first, offline-first, user-data-ownership, and CLI-first are unchanged; this
is a scoping clarification that avoids overstating the local-disk guarantee.

### Note — F1 / #161 (Secret Key in the SRP verifier) shipped

The two-secret SRP verifier described in "Two-Secret Authentication Model" is
implemented as written: `toku_core::srp_verifier_input(secret_key, password)`
(`crates/toku-core/src/crypto/srp.rs`, domain `toku/srp/verifier-input/v1`) folds
the Secret Key into every verifier create/verify site. Issue **#161** (the F1
finding that the Secret Key was initially not folded into the verifier) is
**closed/shipped**, so the threat-model claim that a stolen verifier is not
brute-forceable against the password alone is now true as implemented.

## Context

ADR-006 chose **optional** symmetric encryption (single passphrase → Argon2id → AES-256-GCM)
and an **unauthenticated relay** server, explicitly rejecting zero-knowledge and SRP as
"overkill for book data." ADR-008 defined the wire protocol in that context.

Epic #114 (Immich-style self-hosting with 1Password-style auth) surfaced a fundamental
security gap: the current design has **open registration** — anyone who knows or guesses a
`library_id` can register a device and pull all ops. This is not a theoretical risk; it is
a design flaw that makes self-hosting unsafe.

Three factors have changed since ADR-006 was written:

1. **Self-hosting implies real users.** A Docker image deployed on a VPS faces the public
   internet. An unauthenticated relay is not viable; we need real accounts and credentials.
2. **The "overkill" cost estimate was overstated.** ADR-006 estimated ~3 months for a
   Pildora-style system. Modern Rust crates (`srp`, `argon2`, `aes-gcm`, `p256`, `rand`)
   reduce implementation complexity significantly. The 1Password model, applied to a simpler
   single-user personal tool, is meaningfully cheaper than the full Pildora vault-sharing
   hierarchy.
3. **User data ownership is a non-negotiable.** Mandatory zero-knowledge E2E is a stronger
   guarantee than optional encryption — it removes the server as a potential trust boundary.

## Decision

### Overview

Replace the unauthenticated optional-encryption relay with an **Immich-style self-hostable
server** that uses:

- **1Password-style two-secret SRP authentication**: account password + device-generated
  Secret Key. The server never sees either secret.
- **Mandatory zero-knowledge E2E encryption** for all hosted/sync operations. There is no
  plaintext mode for synced data.
- **Real user accounts** with first-run admin onboarding, device management, and
  authenticated sessions.

The wire protocol defined in ADR-008 (op-log, HLC, UUID v7, cursor contract, batch limits,
snapshots, rate limiting) is **retained**. What changes is the authentication layer, the
trust model of the server, and the encryption posture.

### Two-Secret Authentication Model

Users authenticate with two secrets that are never transmitted to the server:

1. **Account password** — chosen by the user, memorable.
2. **Secret Key** — a high-entropy random value (128-bit, base32-encoded with a `TK`
   version prefix, grouped for readability and ending in a 2-character checksum, e.g.
   `TK-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX-CC`) generated on the user's device at sign-up.
   The checksum lets typos be caught before the key is used. See `docs/recovery.md`.

Authentication uses **SRP (Secure Remote Password, RFC 5054)**. The SRP verifier stored on
the server is derived from both the password and the Secret Key via a domain-separated hash:

```text
verifier_input = SHA-256(domain_sep || Secret Key || account_password)
SRP verifier   = srp::generate_verifier(verifier_input)
```

`domain_sep` is a fixed tag (`toku/srp/verifier-input/v1`) and the Secret Key is
length-prefixed so the key/password boundary is unambiguous. This derivation lives in one
place — `toku_core::srp_verifier_input(secret_key, password)` — and every verifier
create/verify site (CLI account signup/login/migrate, device enrollment, and the hosted
web dashboard login/setup) routes through it so the two auth tiers stay interoperable.

Because the verifier depends on both secrets, a stolen verifier (server DB breach) is not
brute-forceable against the password alone: an attacker must also guess the 128-bit Secret
Key. The server stores only the SRP verifier — it cannot derive the password, the Secret
Key, or any encryption key from it.

> **Single-secret library/passphrase path.** The legacy `toku sync init` passphrase flow
> (identity = `library_id`, no separate Secret Key) routes through the same helper with
> `secret_key = None`, i.e. `SHA-256(domain_sep || passphrase)`. This keeps the derivation
> path uniform but adds no entropy — there is only one secret to fold in.
>
> **Verifier scheme is versioned by the domain tag, not migrated.** Folding the Secret Key
> changes every stored verifier. Sync is a pre-release phase with no deployed accounts, so
> this ships as a clean break: any account enrolled before this change must re-run setup /
> re-enroll. A future scheme change bumps the `…/v1` tag rather than migrating verifiers.

### Key Hierarchy

> **Amended (#197):** The shipped master-unlock-key derivation is a two-step
> construction — Argon2id over the password, then a domain-separated SHA-256
> combine with the Secret Key — not both secrets entering Argon2id together. See
> the 2026-07-27 Amendment (A1) above.

```text
Secret Key + Account Password
          │
          ▼ Argon2id (m=64 MB, t=3, p=1, salt=per-account)
          │
    Account Unlock Key (256-bit)
          │
          ├─── wraps ──► Account Key Pair (X25519 ECDH)
          │                     │
          │                     └─ public key stored on server
          │                        private key stored encrypted on server
          │
          └─── per-library ──► Library Encryption Key (AES-256-GCM)
                                      │
                                      └─ wraps all sync op fields
                                         one key per library
                                         stored encrypted under account key pair
```

Concretely:

- **Account Unlock Key**: derived from (Secret Key + password) via Argon2id. Only exists
  ephemerally in memory during an authenticated session.
- **Account Key Pair**: asymmetric keypair (X25519). Private key is wrapped (AES-256-GCM)
  with the Account Unlock Key and stored on the server. The server stores the ciphertext
  but cannot unwrap it.
- **Library Key**: per-library AES-256-GCM key, generated at library creation. Wrapped
  with the account's public key and stored on the server. The client unwraps it using the
  private key after authentication.

> **Amended (#197):** The wrapped data key is stored **per account**
> (`users.wrapped_data_key`), not per library; a single account's libraries all
> share one data key and are not cryptographically isolated from each other. See
> the 2026-07-27 Amendment (A2) above.

The server stores: SRP verifier, wrapped private key ciphertext, wrapped library key
ciphertext. It **cannot** derive any plaintext data from these.

### Emergency Kit and Recovery

- The Secret Key is surfaced **once** during account creation and presented as an
  **Emergency Kit** — a printable/downloadable document the user keeps offline. Toku
  renders it as plain text, self-contained printable HTML, or PDF
  (`toku account emergency-kit`). See `docs/recovery.md`.
- Losing the Secret Key with no local device means the server data is **unrecoverable**.
  This is documented clearly and intentionally: there is no server-side recovery path.
- **Recovery path**: any local SQLite copy of the library is the recovery. `toku export backup`
  creates a portable archive. Local-first is the safety net.
- **Password change**: user re-authenticates with current secrets, derives new Account Unlock
  Key, re-wraps the private key with the new unlock key, pushes the new wrapped blob.
  The Library Key itself does not change; only the wrapper changes.

### Device Enrollment

New devices are enrolled by entering the Secret Key:

1. User installs Toku on a new device and points it at the server URL.
2. User enters their account email + password + Secret Key.
3. Client performs SRP auth, derives the Account Unlock Key, unwraps the private key,
   decrypts the Library Key.
4. Device generates a device keypair, sends the device public key to the server.
5. Server records the new device. Future ops from this device are authenticated.

There is **no server-side key escrow**. The server cannot enroll devices without the user's
Secret Key.

### Local-First Non-Negotiable Preserved

| Mode | Account Required? | Server Required? | Network Required? |
|------|-------------------|------------------|-------------------|
| Single-device offline | No | No | No |
| Manual backup/restore | No | No | No |
| Multi-device sync (hosted) | Yes | Yes | At sync time |

Single-device, offline-only usage is unchanged. Toku is installed, the local SQLite is
the library, and no account exists. The sync/hosted layer is entirely opt-in.

### Wire Protocol Changes from ADR-008

The op-log format, HLC semantics, cursor contract, batch limits (1000 ops), snapshot format,
and rate limiting are **retained** from ADR-008.

The following change:

| Area | ADR-008 | ADR-010 |
|------|---------|---------|
| Registration | `POST /sync/register` unauthenticated | SRP auth flow + device enrollment; see #120 |
| Session auth | None (library_id + device_id header) | SRP session token; all routes require auth |
| `fields` encryption | Optional (`encrypted` envelope or plaintext) | Mandatory for hosted mode; `encrypted` envelope always present |
| Server trust model | Dumb relay — may store plaintext | Zero-knowledge — stores only encrypted blobs |
| Auth endpoint | None | `POST /auth/login` (SRP exchange); `POST /auth/enroll` (new device) |

### Zero-Knowledge Enforcement (issue #121)

Mandatory encryption is enforced at the **server boundary**, not just by client convention:

- `push` and `rekey` reject the whole batch (HTTP `422 Unprocessable Entity`) if any op
  `payload` is not an encrypted envelope (`{ev, alg, nonce, ciphertext, aad}`) or `null`
  (content-free ops such as deletes). The server requires the *exact* envelope key set, so a
  client cannot smuggle plaintext fields alongside the ciphertext.
- `snapshot` upload rejects any `snapshot_json` that is not a serialized encrypted envelope —
  closing the previous gap where compacted library state was stored in plaintext.
- The client makes encryption mandatory too: `toku sync init` always requires a passphrase
  (the passwordless/plaintext opt-out is removed), `push` refuses to upload without a key, and
  snapshots are encrypted on create / decrypted on bootstrap.

**Server-visible metadata** is minimized to what relay/ordering requires: `op_id`, `device_id`,
`hlc`, `entity_type`, `entity_id`, `op_type`. The `payload` and snapshot blobs are always
ciphertext. `entity_type` and `op_type` are deliberately left cleartext — the client binds them
into the AEAD AAD (so the server cannot re-target or swap an op undetected) and they are needed
for indexing. Making `entity_type` opaque was evaluated and rejected: it would break the AAD
binding and op-type indexing while leaking only aggregate counts, never content. This residual
exposure is documented in `docs/sync-server.md`.

### Revised Threat Model

> **Amended (#197):** "Zero-knowledge / encrypted" here scopes to server-side +
> in-transit + the ZK-wrapped data key. The local `toku.db` is **not** encrypted
> at rest today (WAL + FK pragmas only); SQLCipher at-rest is future work (#204).
> See the 2026-07-27 Amendment (A3) above.

| Threat | Old model (ADR-006/008) | New model (ADR-010) |
|--------|-------------------------|---------------------|
| Server compromise | Attacker reads plaintext ops (if no passphrase) or encrypted blobs | Attacker gets only encrypted blobs, SRP verifiers, wrapped keys — cannot read any data |
| Network interception | Password sent in plaintext (or no auth at all) | SRP: password/Secret Key never cross the wire |
| Unauthorized device registration | Anyone with a `library_id` can register | Requires Secret Key — no server-side path exists |
| Credential brute-force | No server-side credential | SRP verifier folds password **and** the 128-bit Secret Key (`srp_verifier_input`); a stolen verifier is not brute-forceable against the password alone |
| Lost device | Deregister device; all data still readable if encryption was optional | Deregister device; Library Key unreachable from deregistered device's key |
| Passphrase/key forgotten | Server data unrecoverable; local SQLite is recovery | Same: local SQLite is recovery |

### Trade-Offs vs ADR-006 / ADR-008

| Aspect | ADR-006/008 (old) | ADR-010 (new) | Δ |
|--------|-------------------|---------------|---|
| Encryption posture | Optional | Mandatory for hosted mode | Stronger |
| Server auth | Unauthenticated relay | SRP two-secret | Stronger |
| Key complexity | Single derived key | 3-layer hierarchy | Higher |
| Server knowledge | Op count + entity types (if encrypted) | Same, enforced | Same |
| Engineering cost | ~2–3 weeks | ~3 months (epic #114 sub-issues) | Higher |
| User onboarding friction | None | Emergency Kit + Secret Key | Higher |
| Recovery story | Local SQLite (same) | Local SQLite (same) | Same |
| Rationale | Book data < health data | Self-hosting = real attack surface | Justified |

### Why This Cost Is Now Justified

The prior "2–3 weeks" estimate assumed no auth, no accounts, and optional crypto. As
soon as we add self-hosting with Docker and a public-internet server, we need:

- Real user accounts (no unauthenticated access to anyone's library)
- Credential security (passwords that can be verified without storing them)
- Cryptographic guarantees that server operators cannot read user data

These are not optional extras — they are the minimum bar for a self-hosted tool that stores
personal data. The "overkill" judgment in ADR-006 was correct for a local-only tool, but
incorrect for a networked self-hosted server.

## Consequences

- **toku-core**: New `crypto` module for key hierarchy (KDF, key wrapping, ECDH).
  See #116.
- **toku-sync**: New auth layer (SRP), user accounts table, device enrollment flow,
  mandatory encryption enforcement. See #117, #119, #120, #121.
- **toku-cli**: Account signup/login/device enrollment UX, Emergency Kit prompt, Secret
  Key management. See #123.
- **toku-web**: Authenticated sessions via SRP-derived session token, login UI. See #122.
- **Deployment**: Docker image + docker-compose + first-run onboarding. See #124.
- **Security review**: Full threat model audit before Phase 7 merge. See #125.
- **Migration**: Path from the open relay model (existing users, if any). See #126.
- **Wire format compatibility**: ADR-008 op-log format is retained; existing op data can
  be re-encrypted and migrated. See #126.

## Sub-Issues

| Issue | Scope |
|-------|-------|
| #116 | Key hierarchy in `toku-core` |
| #117 | SRP authentication (server/client) |
| #118 | Secret Key generation + Emergency Kit |
| #119 | User accounts, admin, multi-user schema in `toku-sync` |
| #120 | Authenticated device enrollment |
| #121 | Mandatory E2E encryption for hosted mode |
| #122 | `toku-web` login + session auth |
| #123 | CLI account/enrollment UX |
| #124 | Docker image + docker-compose + deployment docs |
| #125 | Threat model security review |
| #126 | Migration from open relay model |

## Alternatives Considered

| Option | Rejected Because |
|--------|-----------------|
| Keep optional encryption, add HTTP Basic Auth | Does not address server-compromise threat; Basic Auth over TLS is weaker than SRP |
| OIDC/OAuth2 (delegate to external IdP) | Adds mandatory external dependency; breaks offline onboarding; does not solve E2E key hierarchy |
| Keep ADR-006/008 for self-hosted, add ZK only for managed | Inconsistent security model; self-hosted users deserve the same guarantees |
| Full Pildora model (vault sharing, per-item keys) | No sharing feature exists; per-item keys are unnecessary complexity for a personal tool |

## Relay migration (issue #126)

Pre-account relay instances upgrade via a one-time `toku sync migrate`: the client generates
a Secret Key + account password, the first account bootstraps as admin and the server adopts
all orphan (unowned) libraries/devices, and all server ops/snapshots are re-keyed from the
single passphrase (or plaintext) into zero-knowledge ciphertext under the new key hierarchy.
The instance then locks `min_protocol = 2`, closing the legacy unauthenticated path. The
migration is forward-only; back up `sync.db` first.
