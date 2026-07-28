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

### Local device at rest (single-device / offline)

The network model above concerns hosted sync. For the **default single-device, offline** use,
the relevant adversary is **offline theft of the device or disk**. By default the local `toku.db`
is **plaintext at rest** — `toku-db` links standard SQLite, so confidentiality depends entirely
on **OS full-disk encryption**.

[ADR-016](../adr/016-at-rest-encryption.md) designed the mitigation and it now **ships**
(issue #225): **optional, off-by-default** at-rest DB encryption (SQLCipher, behind the `toku-db`
`sqlcipher` cargo feature), keyed by a dedicated passphrase through Argon2id (m=64 MiB, t=3, p=1)
— independent of sync. Enable it on an existing library with `toku db encrypt`; check state with
`toku db status`; revert with `toku db decrypt`. When enabled, an offline attacker who takes the
disk sees only ciphertext. The disabled default path is byte-for-byte unchanged, and builds
compiled without the `sqlcipher` feature behave exactly as before.

Key handling: the passphrase and derived key are **never persisted** — only the Argon2id salt,
KDF parameters, and a verifier live in `config.toml`'s `[encryption]` section. Across short-lived
CLI processes the key is resolved by (in order) the opt-in `TOKU_DB_PASSPHRASE` environment
variable, an opt-in OS-keychain cache (`toku db encrypt --remember`), or an interactive prompt
(the baseline). The env var is the weakest option — it can leak via `ps`, shell history, and CI
logs — so prefer the prompt or the keychain. **A lost passphrase makes the database
unrecoverable: there is no backdoor** (see [`recovery.md`](../recovery.md)).

Shipping alongside (from ADR-016 Decision C): **offline-only users can encrypt backups.**
`toku export backup --encrypt` without a sync account derives a key from a passphrase (Argon2id)
and seals the archive with AES-256-GCM; the KDF salt/params travel **inside** the archive so it
restores on any machine with only the passphrase (see [`recovery.md`](../recovery.md)). This does
not protect the live `toku.db` — only the exported artifact — and a lost passphrase makes that
artifact unrecoverable by design.

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
| Read user content | Mandatory E2E; payloads/snapshots are ciphertext; server holds no key | Cleartext metadata (entity/op type, timing) |
| Derive password / Secret Key | Server stores only SRP verifier + wrapped keys; verifier folds password **and** Secret Key (**F1**, fixed); never sees secrets | Offline brute-force must guess password + 128-bit Secret Key |
| Forge ops to a victim | Per-op AAD binds entity_type/entity_id/op_type; tampering breaks AEAD | None for content; metadata reorder possible |
| Tamper key material | Wrapped keys authenticated (AES-256-GCM); unwrap fails on tamper | None |

### Network attacker

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Capture password / Secret Key | SRP — neither crosses the wire; TLS in deployment | TLS not code-enforced — operator responsibility (**F11**, documented) |
| Steal session token | Tokens 256-bit, hashed at rest, 24h TTL, device-scoped; logout/disable revoke server-side (**F6**, fixed) | Un-revoked token valid until 24h TTL |
| MITM via bad cert | reqwest default cert/hostname validation; no `danger_accept_invalid_certs` | Self-signed CA needs config |

### Lost / stolen device

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Read synced library | Device-deregister revokes its sessions; data key unreachable to deregistered device | Un-revoked token valid until 24h TTL (**F6**, fixed — logout available) |
| Read keys on disk | Tokens/derived key in OS keychain; file fallback 0o600 | Plaintext file fallback — documented tradeoff (**F10**) |

### Stolen Emergency Kit (Secret Key only)

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Log in / enroll device | Needs password too, and the verifier folds both secrets (**F1**, fixed) | Kit alone never authenticates; a stolen verifier now also resists brute-force without the password |

### Malicious admin (multi-user)

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Read other users' content | Zero-knowledge: keys wrapped per user; admin gets ciphertext | None for content |
| Disable users / approve devices | First account=admin; no self-disable; user_sessions + device `sessions` revoked on disable (**F6**, fixed); actions audit-logged (**F7**, fixed) | None observed |
| Cross-tenant access | Queries scoped by token-derived user_id/library_id | None observed |

### Malicious client

| Threat | Mitigation | Residual |
|--------|------------|----------|
| Upload plaintext | push/rekey/snapshot reject non-envelope payloads (HTTP 422, exact key-set) | None |
| Cross-library/user | session token bound to library/user; enrollment ownership-checked (403) | None observed |
| Online brute-force | Lockout 5 fails → 15 min per account/library; app-level per-IP + global rate limit (**F8**, fixed, 429) | Lockout `423` still distinguishable from `401` for real accounts (weak oracle) |

## 5. Zero-knowledge verification

- **SRP verifier non-reversibility** — RFC 5054 Group 14 (2048-bit), SHA-256. Server stores
  only salt + verifier; password/Secret Key never transmitted (SRP runs client-side in
  `toku-sync-client`). The verifier input folds both secrets —
  `srp_verifier_input = SHA-256(domain_sep || Secret Key || password)` (**F1**, fixed).
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
Maud auto-escaping (no XSS), fresh token per login (no fixation), security response headers
(CSP/X-Frame-Options/nosniff/Referrer-Policy, HSTS when secure — **F4**, fixed), constant-time
login that no longer reveals unknown vs inactive emails (**F5**, fixed). Remaining by design:
**plaintext password reaches the server** (trusted-server).

## 8. Findings & triage

F1 (HIGH) is now **fixed** under [#161](https://github.com/kafkade/toku/issues/161): the SRP
verifier input folds in the Secret Key via `toku_core::srp_verifier_input`. All other findings
(F2–F11) were addressed under [#160](https://github.com/kafkade/toku/issues/160): the code-level
ones are **fixed** and the operational/dependency ones are **documented**. Residual risks accepted
for release are listed in [§9](#9-residual-risks-accepted).

| ID | Finding | Sev | Disposition |
|----|---------|-----|-------------|
| F1 | SRP verifier input omits Secret Key — diverged from ADR-010 | HIGH | **Fixed** (#161): all verifier create/verify sites derive `SHA-256(domain_sep \|\| Secret Key \|\| password)` via `srp_verifier_input`; web login gained a Secret Key field. Verifier scheme changed — pre-release clean break, existing accounts re-enroll |
| F4 | No web security headers (CSP, X-Frame-Options, nosniff, HSTS) | MED | **Fixed** (#160): `security_headers` middleware in `toku-web`; HSTS when `secure_cookies` |
| F5 | Login/challenge account enumeration | MED | **Fixed** (#160): constant-time web login + phantom-account uniform sync challenge/verify (401) |
| F6 | No token revocation/logout on sync server; user-disable keeps device sessions | MED | **Fixed** (#160): `/auth/logout` + `/account/logout`; disable now purges device `sessions` |
| F7 | No audit logging | MED | **Fixed** (#160): `audit_log` table + `security::audit` on auth failures & admin actions |
| F8 | No IP/global rate limit (proxy-dependent) | MED | **Fixed** (#160): in-process per-IP + global limiter on auth endpoints (429); proxy still recommended |
| F2 | CSPRNG `.expect()` panics on crypto path | LOW | **Fixed** (#160): nonce/salt generation returns `Result`, errors mapped to `TokuError::Crypto` |
| F3 | `srp` pinned to `0.7.0-rc.3` | LOW | **Documented** (#160): no stable 0.7.x exists; exact `=` pin kept, revisit when 0.7.0 ships |
| F9 | Device approvals off by default | LOW | **Documented** (#160): `sync-server.md` recommends enabling on multi-user/internet-facing instances |
| F10 | Token file fallback plaintext at rest | LOW | **Documented** (#160): `recovery.md` Token storage tradeoff + hardening guidance |
| F11 | TLS/perms/at-rest/NTP assumed, not enforced | INFO | **Documented** (#160): `sync-server.md` "Security hardening & operator responsibilities" |

## 9. Residual risks accepted

- Cleartext op metadata (entity/op type, counts, timing) is visible to the server.
- Web dashboard is trusted-server: holds plaintext password (login) and decrypted library.
- Session token window before expiry: logout/disable now revoke server-side sessions (F6), but
  an un-revoked token remains valid until its 24h TTL.
- **F5**: the account-lockout response (`423`) is still distinguishable from a normal auth
  failure (`401`) for a *real* account under active brute-force, a weak residual oracle; the
  challenge/verify path itself is uniform for unknown/disabled accounts.
- **F8**: the built-in limiter is coarse (fixed-window, per-process) and shares one bucket for
  clients behind the same proxy hop; a rate-limiting reverse proxy remains the primary control.
- **F10**: the client token file fallback is plaintext protected only by `0600` perms; mitigated
  by at-rest disk encryption on headless hosts.
- Deployment must still provide TLS, 0600 DB perms, at-rest encryption, and NTP (F11); these are
  operator responsibilities documented in `sync-server.md`.
- The local `toku.db` is plaintext at rest by default; single-device confidentiality relies on OS
  full-disk encryption. Optional at-rest DB encryption
  ([ADR-016](../adr/016-at-rest-encryption.md), opt-in, off by default) now ships (#225) behind the
  `toku-db` `sqlcipher` feature — enable it with `toku db encrypt`. Offline passphrase-encrypted
  backups also ship (#204). A lost DB passphrase is unrecoverable by design (no backdoor).

## 10. Sign-off

The CLI/native sync path is **zero-knowledge**: no plaintext password, Secret Key, or library
content reaches the `toku-sync` server — verified by client-side SRP, ciphertext-only payloads,
and server-side 422 plaintext rejection. The hosted web dashboard is a documented
**trusted-server** exception. The former divergence from ADR-010 (**F1**, verifier omitted the
Secret Key) has been resolved under #161 — the verifier now folds both secrets, restoring the
128-bit Secret Key's contribution to credential strength. The remaining findings (F2, F4–F11)
have been resolved under #160 — code fixes for F2/F4/F5/F6/F7/F8 and documented
operator/dependency guidance for F3/F9/F10/F11 — with the accepted residual risks recorded in
[§9](#9-residual-risks-accepted).
