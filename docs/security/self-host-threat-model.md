# Self-Hosted Auth Threat Model & Security Review

**Status**: Draft for sign-off
**Scope**: The self-hosted, multi-user, zero-knowledge sync/auth model defined by
[ADR-010](../adr/010-self-host-auth.md) and implemented in issues #116–#123.
**Supersedes**: the ADR-006 relay threat model ("server knows op count, timestamps,
entity types; standard auth or passphrase only").
**Tracking issue**: #125 (epic #114).

This document is the end-to-end threat model and security-review record for Toku's move from
a "dumb relay + optional encryption" sync server to a 1Password-style, two-secret,
zero-knowledge, multi-user self-host model. It enumerates assets, adversaries, and
mitigations; verifies the zero-knowledge guarantee; reviews key handling and the web tier;
triages the security-review findings; and records residual risks and the sign-off.

> This review is **documentation-only**. Code remediations are tracked as follow-up issues
> (see [Findings & triage](#8-findings--triage)). No source was changed for this deliverable.

## 1. System overview & trust boundaries

Three deployment shapes share one core library:

- **Single-device, offline (default)** — local SQLite only, no account, no network. Out of
  scope for the network threat model; covered only by local-device assets.
- **CLI / native sync client** (`toku-core`, `toku-sync-client`, `toku-cli`, `toku-ffi`) —
  performs SRP and all encryption/decryption locally. **True zero-knowledge.**
- **Hosted web dashboard** (`toku serve`, `toku-web`) — renders server-side HTML from the
  decrypted library, so the process necessarily holds plaintext. **Trusted-server posture**
  (documented in [`web-auth.md`](../web-auth.md)).

The sync server (`toku-sync`) is the shared, internet-facing component.

```text
┌───────────────┐    SRP login (no pw)    ┌──────────────────────┐
│  CLI / native │◄──────TLS───────────────►│   toku-sync server   │
│  client       │  ciphertext ops only     │  (zero-knowledge)    │
└──────┬────────┘                          │  stores: SRP verifier│
       │ keychain                          │  wrapped keys, cipher│
       ▼                                   │  text ops/snapshots  │
   OS secret store                         └──────────┬───────────┘
                                                      │ SQLite (encrypted blobs)
┌───────────────┐  pw over TLS (trusted)              ▼
│  web dashboard │◄──────TLS──────────────────────► holds plaintext in-process
└───────────────┘
```

Trust boundaries: client device ↔ network ↔ server process ↔ server disk; plus, in
multi-user mode, user ↔ admin and user ↔ user.

## 2. Assets

| Asset | Where it lives | Must never reach server |
|-------|----------------|-------------------------|
| Account password | User memory; client RAM transiently | Yes |
| Secret Key (128-bit) | Emergency Kit (offline); client RAM transiently | Yes |
| Account Unlock Key | Client RAM only, ephemeral | Yes |
| Account key pair (X25519 private) | Wrapped under unlock key; server holds ciphertext | Plaintext: yes |
| Library/data key (AES-256-GCM) | Wrapped under account key pair | Plaintext: yes |
| Op content (book/reading data) | Local SQLite; ciphertext payloads on server | Plaintext: yes |
| SRP verifier + salt | Server DB | n/a (non-reversible) |
| Session tokens | Server DB (hashed); client keychain | n/a |
| Metadata (op_id, device_id, hlc, entity_type, op_type) | Server DB cleartext | Accepted exposure |

The web dashboard additionally holds the **decrypted library** and the **plaintext password
at login** in process memory — by design.

## 3. Adversaries

1. **Malicious / compromised server operator** — full read/write of server DB and process.
2. **Network attacker (MITM)** — observes/modifies traffic between client and server.
3. **Lost / stolen device** — attacker has an enrolled device, possibly unlocked.
4. **Stolen Emergency Kit** — attacker has the Secret Key but not the password.
5. **Malicious admin (multi-user)** — legitimate admin abusing privileges against other users.
6. **Malicious client** — attempts to smuggle plaintext, cross tenants, or replay.

## 4. Threats, mitigations & residual risk

### Malicious / compromised server

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Read user content | Mandatory E2E; payloads/snapshots are ciphertext; server holds no key | Cleartext metadata (entity/op type, timing); see F1 for verifier note |
| Derive password / Secret Key | Server stores only SRP verifier + wrapped keys; never sees secrets | Offline brute-force of verifier — weaker than intended; **F1** |
| Forge ops to a victim | Per-op AAD binds entity_type/entity_id/op_type; tampering breaks AEAD | None for content; metadata reorder possible |
| Tamper key material | Wrapped keys authenticated (AES-256-GCM); unwrap fails on tamper | None |

### Network attacker

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Capture password / Secret Key | SRP — neither crosses the wire; TLS in deployment | TLS not code-enforced (**F11**) |
| Steal session token | Tokens 256-bit, hashed at rest, 24h TTL, device-scoped | No revocation (**F6**) |
| MITM via bad cert | reqwest default cert/hostname validation; no `danger_accept_invalid_certs` | Self-signed CA needs config |

### Lost / stolen device

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Read synced library | Device-deregister revokes its sessions; data key unreachable to deregistered device | 24h token window (**F6**) |
| Read keys on disk | Tokens/derived key in OS keychain; file fallback 0o600 | Plaintext file fallback (**F10**) |

### Stolen Emergency Kit (Secret Key only)

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Log in / enroll device | Needs password too; password not in kit | **F1**: Secret Key not in verifier — kit alone never authenticates, but auth strength rests on password |

### Malicious admin (multi-user)

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Read other users' content | Zero-knowledge: keys wrapped per user; admin gets ciphertext | None for content |
| Disable users / approve devices | First account=admin; no self-disable; user_sessions revoked | Device `sessions` survive disable (**F6**); no audit trail (**F7**) |
| Cross-tenant access | Queries scoped by token-derived user_id/library_id | None observed |

### Malicious client

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Upload plaintext | push/rekey/snapshot reject non-envelope payloads (HTTP 422, exact key-set) | None |
| Cross-library/user | session token bound to library/user; enrollment ownership-checked (403) | None observed |
| Online brute-force | Lockout 5 fails → 15 min per account/library | No IP/global limit (**F8**) |

## 5. Zero-knowledge verification

- **SRP verifier non-reversibility** — RFC 5054 Group 14 (2048-bit), SHA-256. Server stores
  only salt + verifier; password/Secret Key never transmitted (SRP runs client-side in
  `toku-sync-client`). **Caveat:** verifier input omits the Secret Key — see **F1**.
- **Ciphertext-only payloads** — all op payloads and snapshots are `{ev,alg,nonce,ciphertext,aad}`
  envelopes; nonces are 96-bit random (uniqueness asserted over 10k ops).
- **Server-side enforcement** — `push`/`rekey`/`snapshot` reject anything that is not the exact
  envelope key-set (or `null` for content-free ops) with **422**, preventing plaintext smuggling.
- **Minimized metadata** — only `op_id`, `device_id`, `hlc`, `entity_type`, `op_type` are cleartext;
  `entity_type`/`op_type` are bound into AAD so the server cannot re-target ops. Aggregate counts
  and timing remain visible — accepted, documented in [`sync-server.md`](../sync-server.md).

## 6. Key handling review

- **Secret Key** — 128-bit OsRng, base32 + checksum; never transmitted; never persisted; shown
  once via Emergency Kit.
- **Zeroization** — `SecretKey`, `SyncKey`, `MasterUnlockKey` are `ZeroizeOnDrop`; Argon2 output
  and ECDH wrap keys zeroized after use.
- **KDF/wrap** — Argon2id (m=64 MB, t=3, p=1, 128-bit salt) → unlock key; AES-256-GCM wraps the
  private key; X25519 ECDH (ephemeral) wraps the data key.
- **At rest** — client tokens/keys in OS keychain by default; file fallback is 0o600 plaintext.
  Server stores only verifiers, wrapped keys, ciphertext.

## 7. Web tier review

CSRF (double-submit, constant-time, SameSite=Strict cookie), session cookies (HttpOnly, Lax,
Secure-conditional, 24h, hashed at rest), lockout (5/15 min), logout revokes the session,
Maud auto-escaping (no XSS), fresh token per login (no fixation). Gaps: **no security headers**
(**F4**), **plaintext password reaches the server** (trusted-server, by design), enumeration
on unknown email (**F5**).

## 8. Findings & triage

All code fixes are deferred to follow-ups (doc-only PR). Residual risks accepted for release
are listed in [§9](#9-residual-risks-accepted).

| ID | Finding | Sev | Disposition |
|----|---------|-----|-------------|
| F1 | SRP verifier input omits Secret Key (`orchestrator.rs:819`, `auth.rs:141/190/291`) — diverges from ADR-010 | HIGH | Bug; follow-up #161 |
| F4 | No web security headers (CSP, X-Frame-Options, nosniff, HSTS) | MED | Follow-up #160 |
| F5 | Login/challenge account enumeration | MED | Follow-up #160 |
| F6 | No token revocation/logout on sync server; user-disable keeps device sessions | MED | Follow-up #160 |
| F7 | No audit logging | MED | Follow-up #160 |
| F8 | No IP/global rate limit (proxy-dependent) | MED | Follow-up #160 |
| F2 | CSPRNG `.expect()` panics on crypto path | LOW | Follow-up #160 |
| F3 | `srp` pinned to `0.7.0-rc.3` | LOW | Follow-up #160 |
| F9 | Device approvals off by default | LOW | Follow-up #160 |
| F10 | Token file fallback plaintext at rest | LOW | Follow-up #160 |
| F11 | TLS/perms/at-rest/NTP assumed, not enforced | INFO | Follow-up #160 |

## 9. Residual risks accepted

- **F1**: until fixed, a stolen SRP verifier is offline-brute-forceable against the password
  alone; the Secret Key's 128 bits do not harden authentication. Content stays zero-knowledge.
- Cleartext op metadata (entity/op type, counts, timing) is visible to the server.
- Web dashboard is trusted-server: holds plaintext password (login) and decrypted library.
- 24h session window before token expiry (no revocation yet).
- Deployment must provide TLS, 0600 DB perms, at-rest encryption, NTP, and a rate-limiting proxy.

## 10. Sign-off

The CLI/native sync path is **zero-knowledge**: no plaintext password, Secret Key, or library
content reaches the `toku-sync` server — verified by client-side SRP, ciphertext-only payloads,
and server-side 422 plaintext rejection. The hosted web dashboard is a documented
**trusted-server** exception. The only divergence from ADR-010 (**F1**, verifier omits Secret
Key) does not break the zero-knowledge content guarantee but weakens credential strength and is
tracked for fix. All other findings are deferred with residual risks documented above.
