# ADR-010: Self-Hosted Server with Mandatory Zero-Knowledge Encryption and 1Password-Style SRP Authentication

**Status**: Accepted
**Date**: 2026-06-26
**Supersedes**: ADR-006, ADR-008
**Issue**: #114 (Epic), #115 (this ADR)

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
the server is derived from both the password and the Secret Key:

```text
verifier_input = hash(Secret Key || account_password)
SRP verifier   = srp::generate_verifier(verifier_input)
```

The server stores only the SRP verifier — it cannot derive the password, the Secret Key,
or any encryption key from it.

### Key Hierarchy

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

### Revised Threat Model

| Threat | Old model (ADR-006/008) | New model (ADR-010) |
|--------|-------------------------|---------------------|
| Server compromise | Attacker reads plaintext ops (if no passphrase) or encrypted blobs | Attacker gets only encrypted blobs, SRP verifiers, wrapped keys — cannot read any data |
| Network interception | Password sent in plaintext (or no auth at all) | SRP: password/Secret Key never cross the wire |
| Unauthorized device registration | Anyone with a `library_id` can register | Requires Secret Key — no server-side path exists |
| Credential brute-force | No server-side credential | SRP verifier is memory-hard (Argon2id); Secret Key adds 128 bits of entropy |
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
