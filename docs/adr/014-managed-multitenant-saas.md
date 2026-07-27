# ADR-014: Managed Multi-Tenant SaaS (Relay-Only)

**Status**: Proposed
**Date**: 2026-07-27
**Relates**: ADR-010 (zero-knowledge sync + SRP), ADR-013 (local identity & key bootstrap), ADR-016 (at-rest encryption, proposed in #204 — in review)
**Issue**: #203 (this ADR) · epic #207 (seq 11)

> **Numbering note.** ADR-014 is taken by this issue (#203). ADR-015 is reserved for the
> auth-coherence ADR (#205) and ADR-016 for at-rest encryption (#204, currently in review);
> both are referenced here as **Proposed** siblings, not as merged decisions. On the default
> branch the highest accepted ADR remains 013.
>
> **Scope note.** This is an **architecture and scope** ADR. It is deliberately silent on
> monetization: no pricing, tiers, free-tier limits, revenue, or go-to-market. Where a managed
> operator would attach a billing system, this ADR documents the **structural seam** (where the
> check lives and what it gates), never a commercial offer. When in doubt the text describes the
> technical extension point and omits the commercial angle.

## Context

Toku is a **local-first, offline-first** personal book manager. Its non-negotiables are fixed:
the app works fully offline, has no social features, gives the user complete ownership of their
data in open formats, treats import as a first-class feature, and is CLI-first. Sync and any
hosted tier are **optional** and never required for core use.

Two prior decisions frame this one:

- **ADR-010** established the self-hosted `toku-sync` relay: mandatory **zero-knowledge (ZK)**
  encryption, 1Password-style two-secret auth (account password + device-generated Secret Key)
  over SRP-6a, and the principle that single-device offline use needs no account and no server.
- **ADR-013** formalized the **local identity & key bootstrap** and first-opt-in upload
  semantics that sit on either side of the opt-in boundary.

What this ADR adds is a scope decision that has not been recorded: **can kafkade operate a
managed, multi-tenant tier of `toku-sync`, and if so, within what architectural boundary?**

### What already exists

The relay is *already* multi-tenant and its tenant isolation is tested:

- **User/tenant schema.** `crates/toku-sync/migrations/V5__users_and_admin.sql` defines
  `users` (id, email, `role` ∈ {admin, user}, `status`, and **only** an SRP `srp_salt` +
  `srp_verifier` — never a password), a single-row `instance_config` with `registration_open`
  **defaulting to 0 (closed)**, and ownership FKs `libraries.user_id` / `devices.user_id`
  (`V5:67-68`). `V8__relay_migration.sql` adds the backfill + protocol gate.
- **First-account-becomes-admin, registration-closed-by-default.**
  `crates/toku-sync/src/auth.rs:838-863`: the first signup on a fresh instance bootstraps as
  `admin` regardless of the flag; subsequent signups require an admin to open registration.
  Tested in `tests/user_accounts.rs:196` (`first_signup_bootstraps_admin`) and `:207`
  (`closed_registration_then_open`).
- **Zero-knowledge enforcement.** `crates/toku-sync/src/handlers.rs:45-93`
  (`require_ciphertext_*`) plus the push guard at `handlers.rs:845-846` reject any op or
  snapshot that is not an encrypted envelope, returning HTTP **422** via
  `SyncError::PlaintextRejected` (`error.rs:54`). The relay persists ciphertext only, proven
  white-box in `tests/zero_knowledge.rs`
  (`server_accepts_ciphertext_and_stores_it_opaquely`, plus plaintext push/snapshot → 422).
- **Cross-tenant isolation.** Enrolling into another account's library is forbidden
  (`tests/device_enrollment.rs:327`, `cannot_enroll_into_foreign_library`); device management
  is strictly user-scoped (`:441`, "Bob cannot delete the admin's device"); non-admins cannot
  reach admin endpoints (`tests/user_accounts.rs:288`, → 403). The ownership-check pattern is
  `handlers.rs:800-808`.

In short: **the relay's ZK guarantee holds under a third-party operator.** A vendor running
`toku-sync` stores only client-encrypted ciphertext and cannot read user content, because the
handlers reject plaintext outright.

### What a managed tier would additionally need

The current server is built and documented for **self-hosting** — one operator who is also the
user, or a small trusted group (Immich-style, invite-gated). Running it as a **managed service
for untrusted strangers at scale** surfaces gaps that self-hosting never had to solve:

- **No per-user quotas.** There is a per-request batch cap (`MAX_BATCH_SIZE = 1000`,
  `handlers.rs:19`), but no per-account storage-byte or op-count ceiling.
- **Rate limiting is per-IP + global only.** `crates/toku-sync/src/security.rs:115-193`
  implements a fixed-window limiter keyed by client IP with a global ceiling
  (`DEFAULT_PER_IP_MAX = 60`, `DEFAULT_GLOBAL_MAX = 600`), attached to the public/auth router
  only (`lib.rs:90-100`). There is no limiter keyed by authenticated user.
- **No signup-at-scale primitives.** Signup validates the email's *shape* only
  (`auth.rs:800-802`); there is no email/SMTP verification and no anti-abuse (captcha, velocity
  limits). Self-serve registration at scale needs both.
- **No billing/plan-enforcement hooks.** The server has no notion of a plan or an external
  billing system; `config.rs:6-21` exposes only port/bind/data-dir/log-level.
- **No per-user server-side backup.** Persistence is whole-volume: a single Docker named volume
  `toku-sync-data:/data` (`docker-compose.yml:12-13,25-26`, `Dockerfile` `VOLUME ["/data"]`).
  Operator backup today means "copy the volume," which is coarse and not per-tenant.

### The trust boundary that constrains everything

The relay is ZK and therefore safe to operate for others. **`toku-web` is not.** The web
dashboard is a **trusted-server** design: `docs/web-auth.md:90-107` states plainly that in
hosted mode it "renders **server-side HTML from your decrypted local library**, so the server
process necessarily holds plaintext," and that a true in-browser ZK unlock (client-side WASM
crypto) is explicitly out of scope for the initial dashboard. The handler confirms it opens the
plaintext SQLite library directly (`crates/toku-web/src/library_handlers.rs:167,236,264`, via
`Database::open_no_migrate`).

This split is the spine of this ADR: a managed operator can run the relay without ever seeing
plaintext, but running `toku-web` on a user's behalf would place the user's decrypted library
and login password inside the operator's trust boundary — destroying the zero-knowledge
guarantee that makes a managed offering acceptable at all.

## Decision

Toku **may** offer an optional kafkade-operated managed tier, subject to the following
architectural decisions. The tier is optional; it changes nothing for offline, self-hosted, or
CLI-only users, and no core feature depends on it.

### D1 — Relay-only constraint (the spine)

A managed kafkade tier runs **only** the zero-knowledge `toku-sync` relay. kafkade **must never
run `toku-web` on a user's behalf.**

Rationale: `toku-web` decrypts server-side and holds plaintext (`docs/web-auth.md:90-107`;
`library_handlers.rs:167,236,264`). Hosting it for a user would move that user's decrypted
library and their login password into the operator's trust boundary, breaking the ZK property
the relay is designed to preserve. The relay, by contrast, rejects plaintext (422) and stores
ciphertext only — so it is the *only* Toku server surface a vendor may operate for third parties.

This is a hard boundary, not a default. Every subsequent decision (D2–D7) is scoped to the
relay and preserves ZK.

> **Future work (not a deliverable of this ADR).** A browser-local **WASM client** that performs
> the unlock and all crypto client-side — the server never seeing plaintext — could offer a web
> UX while preserving ZK, and is the correct long-term path to a hosted "web app." That is
> explicitly out of scope here; the current server-rendered dashboard cannot be hosted by the
> operator without breaking ZK, and this ADR does not authorize doing so.

### D2 — Per-user quotas (relay enforcement seam)

The managed tier enforces **per-account ceilings** — total stored ciphertext bytes and/or total
op count per user — in addition to today's per-request `MAX_BATCH_SIZE`.

Enforcement seam: the ingest path (`push_ops`, `handlers.rs:830`) and the snapshot upload path
already run inside the authenticated, ownership-scoped handler where the acting `user_id` /
`library_id` is known. A quota check attaches there, consulting a per-user usage counter
(derivable from `ops` / `snapshots` sizes keyed by ownership) against a ceiling, and rejecting
over-quota writes with a dedicated error (e.g. HTTP 413/429-class) before persistence. Because
the check reads only ciphertext sizes and counts — never plaintext — it does not weaken ZK.

The ceiling *value* is a configuration/plan input (see D5); this decision fixes only **where**
the ceiling is enforced, not its number.

### D3 — Per-user rate limiting

The managed tier adds a **per-authenticated-user** rate limiter layered above today's per-IP +
global limiter (`security.rs:115-193`, wired at `lib.rs:90-100`).

Per-IP limiting is insufficient at scale: many users share an IP (NAT, mobile carriers), and one
abusive account behind a rotating-IP pool evades an IP bucket entirely. The seam is the same
middleware stack: an additional `from_fn` layer on the **authenticated** router (`lib.rs:30-46`)
keys its window on the resolved `AuthUser` / `AuthDevice` identity rather than the socket IP,
returning the existing 429 (`SyncError::RateLimited`). The per-IP/global limiter remains as
defense-in-depth on the pre-auth surface (signup, SRP challenge).

### D4 — Self-serve signup at scale

The managed tier introduces **email verification** and **anti-abuse** as technical capabilities
the self-hosted relay never needed.

- **Email/SMTP verification.** Signup (`auth.rs:813`) currently only checks the email's shape
  (`:800-802`). Self-serve registration requires proving control of the address: issue a signed,
  expiring verification token, deliver it over SMTP, and gate account activation on confirmation.
  This is additive and does not touch the ZK path — the verification token is account metadata,
  not library content.
- **Anti-abuse.** Captcha / proof-of-work on the signup endpoint, per-IP and per-email velocity
  caps, and reuse of the existing audit log (`migrations/V9__security_hardening.sql`,
  `audit_log`) plus the phantom-credential enumeration defense (`security.rs:77-86`).

No pricing or eligibility framing attaches here; this decision is purely the signup *mechanism*.

### D5 — Billing / plan-enforcement integration seams

The managed tier defines **structural extension points** where an external billing/plan system
would attach. This ADR documents the seams only; it specifies no prices, tiers, or commercial
terms.

- **Plan lookup seam.** A per-account "plan/entitlement" record (external system of record;
  cached server-side) that the D2 quota check and D3 rate limiter consult to resolve the ceiling
  for a given user. The enforcement points already exist (D2/D3); this seam is the input they
  read.
- **Billing-state sync seam.** A webhook/ingest endpoint (admin-scoped, outside the ZK data
  path) through which an external billing system updates an account's entitlement state
  (active / past-due / cancelled). This maps naturally onto the existing account `status`
  machinery (`users.status`, admin enable/disable at `handlers.rs:1364`).
- **Capability mapping seam.** A pure mapping from entitlement state → server-side capability
  ceilings (quota bytes, op ceiling, rate window, device count). Enforcement is server-side and
  authoritative; the client is never trusted to self-report its plan.

Explicitly **out of scope**: any price, tier definition, free-tier limit framed as a price,
revenue model, or go-to-market. Those are product/commercial decisions recorded elsewhere, not
in this architecture ADR.

### D6 — Per-user server-side encrypted backup

The managed tier provides **per-user backup of ciphertext**, replacing the self-hosted
"copy the whole `/data` volume" model (`docker-compose.yml:25-26`, `Dockerfile` `VOLUME`).

Because the relay only ever holds encrypted envelopes (`tests/zero_knowledge.rs`), a per-tenant
backup is a backup of **ciphertext + opaque metadata**, scoped by ownership (`user_id` /
`library_id`). The operator can snapshot, replicate, and restore a single account's encrypted
ops/snapshots without ever possessing a key — ZK is preserved end-to-end, including in the
backup system. This is strictly better than whole-volume copy for a multi-tenant deployment:
per-account isolation, per-account restore, and no cross-tenant blast radius. Client-held keys
remain the only path to plaintext; a backup restore returns the user to encrypted state, exactly
as the live relay holds it. (This complements, and does not replace, the client-side canonical
backup of ADR-012, which is the user's own plaintext export.)

### D7 — Managed-tier threat model & metadata leakage

Zero-knowledge protects **content**, not **metadata**. This ADR documents honestly what a
managed operator **can** still observe, so the guarantee is not oversold. Even under ZK, a
managed operator observes:

- **Ciphertext sizes.** Envelope byte lengths for each op and snapshot (leaks approximate record
  size and, in aggregate, library size).
- **Op counts & timing.** Number of ops, push/pull frequency, and timestamps — revealing when
  and how actively a user reads and edits.
- **Device counts & enrollment events.** How many devices an account has and when they enroll,
  approve, or are removed (`devices`, `audit_log`).
- **Signup email.** The account handle is plaintext by necessity (`users.email`) and is required
  for D4 verification.
- **IP addresses & connection metadata.** Client IPs (including `X-Forwarded-For` first hop,
  `security.rs:199-213`), TLS/HTTP metadata, and request patterns, some of which land in the
  audit log.

What the operator **cannot** observe: book titles, authors, notes, ratings, reading sessions, or
any library field — all of which are AEAD-encrypted client-side and rejected in plaintext (422).
It also never holds the account password, Secret Key, or any wrapping key.

Mitigations are acknowledged but not mandated here (e.g. padding envelope sizes, batching to blur
timing, minimizing audit-log retention); the honest disclosure is the decision. A managed
operator must publish this metadata-exposure profile so users can make an informed opt-in — which
is consistent with Toku's data-ownership ethos.

## Consequences

### Positive

- Records a clear, defensible boundary: a vendor can operate the ZK relay for strangers without
  the ability to read their libraries, and the one server surface that would break that
  (`toku-web`) is ruled out (D1).
- The heavy lifting already exists — multi-tenant schema, tested isolation, ZK enforcement — so
  the managed tier is largely *additive seams* (quota, per-user rate limit, signup, backup) on a
  proven base.
- Keeps Toku's non-negotiables intact: the managed tier is optional; offline, self-hosted, and
  CLI-only paths are unchanged and remain first-class.

### Negative / costs

- New server-side surfaces (SMTP, anti-abuse, per-user counters, billing webhooks) add
  operational and security burden that self-hosting never carried.
- Metadata leakage (D7) is real and must be disclosed; ZK is not "the operator sees nothing."
- A genuinely web-based managed UX must wait for a WASM ZK client (D1 future work); the current
  dashboard cannot be offered as a hosted service without breaking ZK.

## Alternatives Considered

- **Host `toku-web` for users (rejected).** The obvious "give users a web app" path. Rejected
  because the dashboard decrypts server-side and holds plaintext + the login password
  (`docs/web-auth.md:90-107`; `library_handlers.rs:167,236,264`), collapsing the ZK boundary the
  managed tier depends on. The relay-only constraint (D1) exists precisely to forbid this.
- **Keep per-IP-only rate limiting (rejected).** Adequate for self-hosting; inadequate at scale
  where IPs are shared or rotated. D3 adds per-user limiting while keeping per-IP as
  defense-in-depth.
- **Whole-volume backup only (rejected for managed).** Fine for a single self-hosted instance,
  but a multi-tenant service needs per-account isolation and restore (D6).
- **Bake plan/pricing logic into the relay (rejected).** Couples the open-source server to a
  commercial model and this public repo to monetization detail. D5 instead defines neutral seams
  an external billing system attaches to, keeping pricing out of the codebase and out of this ADR.
- **Do nothing / self-host only (viable fallback).** Toku remains fully usable self-hosted or
  offline; this ADR does not commit to launching a managed tier, only to the architecture one
  would use if it does.

## CI / Terraform note

This ADR is **docs-only** and adds no CI job. The merge gate remains `Validate`
(`.github/workflows/validate.yml`), whose name is mirrored in the branch-protection IaC
(`kafkade/github-infra`, `repo_toku.tf`); nothing here renames, adds, or removes a gating job, so
no Terraform change is required. Any *implementation* of D2–D6 would arrive under later sequences
(e.g. the seq-12 hardening work, #206) and would be responsible for its own CI/IaC coordination
at that time.

## References

Verified against the tree on this branch:

- Multi-tenant schema: `crates/toku-sync/migrations/V5__users_and_admin.sql`
  (users/instance_config/ownership FKs, `:67-68`); relay-migration gate
  `migrations/V8__relay_migration.sql`; hardening/audit `migrations/V9__security_hardening.sql`.
- First-admin + closed registration: `crates/toku-sync/src/auth.rs:838-863`; signup email-shape
  check `:800-802`; tests `crates/toku-sync/tests/user_accounts.rs:196,207,288`.
- Zero-knowledge enforcement: `crates/toku-sync/src/handlers.rs:45-93,845-846`;
  `src/error.rs:54`; `crates/toku-sync/tests/zero_knowledge.rs`.
- Cross-tenant isolation: `crates/toku-sync/tests/device_enrollment.rs:327,441`; ownership check
  `crates/toku-sync/src/handlers.rs:800-808`.
- Rate limiting (per-IP + global): `crates/toku-sync/src/security.rs:115-213`, wired at
  `crates/toku-sync/src/lib.rs:90-100`; per-request batch cap `handlers.rs:19`.
- Persistence / backup: `docker-compose.yml:12-13,25-26`; `crates/toku-sync/Dockerfile`
  (`VOLUME ["/data"]`); config knobs `crates/toku-sync/src/config.rs:6-21`.
- Trusted-server posture forcing relay-only: `docs/web-auth.md:90-107`;
  `crates/toku-web/src/library_handlers.rs:167,236,264`.
- Related ADRs: `docs/adr/010-self-host-auth.md`, `docs/adr/012-canonical-lossless-backup.md`,
  `docs/adr/013-local-identity-key-bootstrap.md`; proposed siblings ADR-015 (#205) and ADR-016
  (#204, at-rest encryption, in review).
