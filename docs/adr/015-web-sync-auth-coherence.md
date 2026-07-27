# ADR-015: Web + Sync Auth Coherence (Two Account Stores, One Shared Derivation)

**Status**: Proposed
**Date**: 2026-07-27
**Relates**: ADR-010 (self-host auth: zero-knowledge sync + two-secret SRP), ADR-013 (local
identity & key bootstrap), ADR-014 (managed multi-tenant SaaS, relay-only), ADR-016 (at-rest
encryption)
**Issue**: #205 (this ADR) · epic #207 (seq 13, final)

> **Numbering note.** This ADR takes **015** per its tracking issue (#205). ADR-014 (managed
> multi-tenant SaaS, #203) and ADR-016 (at-rest encryption, #204) are already on `main`, and both
> explicitly reserved 015 for this auth-coherence work
> (`docs/adr/014-managed-multitenant-saas.md:8-11`, `docs/adr/016-at-rest-encryption.md:8-11`).
> The highest **Accepted** ADR on the default branch remains 013; 014, 015, and 016 are Proposed
> siblings.
>
> **Scope note.** This is a **docs-only architecture ADR**. It records a decision about two
> existing account stores; it changes **no code, no schema, and no migration**. It is silent on
> monetization and product packaging — it describes only the technical trust boundary and the
> non-breaking paths that could converge the two stores later.

## Context

Toku is **local-first and offline-first**: the whole library works with no account, no server,
and no network. Two *optional* surfaces add authenticated, multi-account behaviour on top of that
core — and they were built independently, so Toku today has **two separate account stores** that
share only a single password-hardening function.

### Store 1 — Web dashboard (`toku serve --hosted`): trusted-server

- **Schema & location.** `web_users` and `web_sessions`, created in
  `crates/toku-db/migrations/V16__web_auth.sql:13,33`, live in the **local `toku.db`** alongside
  the library. `web_users` stores an SRP salt + verifier (never the password), opaque wrapped key
  material, a `role` (`admin`/`user`), lockout counters, and a `created_at`; `web_sessions` holds
  SHA-256 hashes of cookie session tokens.
- **Authentication is a server-side verifier *recompute*, not an SRP handshake.** Login sends the
  email, password, **and** Secret Key to the server over TLS; the server recomputes the verifier
  via `compute_verifier_hex(email, password, secret_key, salt)` and constant-time compares it to
  the stored verifier (`crates/toku-web/src/auth.rs:143-146,348-350`). V16 defines **no challenge
  table** — there is no SRP challenge/response, so the server transiently holds the plaintext
  password and Secret Key.
- **This is deliberate: the tier is trusted-server.** In hosted mode the dashboard renders
  **server-side HTML from the decrypted local library**, so the process necessarily holds
  plaintext (`crates/toku-web/src/auth.rs:13-22`; `docs/web-auth.md:90-107`). Receiving the
  secrets to verify a login is consistent with a surface that already decrypts and renders the
  library.

### Store 2 — Sync server (`toku-sync`): zero-knowledge

- **Schema & location.** `users`, `user_srp_challenges`, and `user_sessions`, created in
  `crates/toku-sync/migrations/V5__users_and_admin.sql:11,44,56`, live in the server's separate
  **`sync.db`**. The `users` row shape mirrors `web_users` (email, SRP salt + verifier, wrapped
  key material, role, status, lockout).
- **Authentication is a full zero-knowledge SRP-6a handshake.** The client runs
  `POST /auth/challenge` + `POST /auth/verify`; the server generates an ephemeral `B`, stores the
  single-use challenge, and verifies `M1` with `ServerG2048` / `process_reply`
  (`crates/toku-sync/src/auth.rs:407-635`). The server **only ever stores** the salt + verifier
  and **never receives the password or Secret Key** — the property ADR-010 requires of the relay.

### The single shared seam

The two stores share exactly **one** piece of code: the verifier-input derivation
`srp_verifier_input(secret_key, password) -> [u8; 32]` in
`crates/toku-core/src/crypto/srp.rs:40-58`, which computes
`SHA-256(domain_sep || len(secret_key) || secret_key || password)` (ADR-010 two-secret hardening).
Both tiers fold the password and Secret Key through it before computing their verifier.

Everything else is **independent**:

- **Salts** — each store generates its own random 16-byte salt, so the same `(password, Secret
  Key)` yields *different* verifiers in each store.
- **Verifiers** — stored in different rows, in different databases.
- **Session tables** — `web_sessions` vs `user_sessions`, different token schemes.
- **Storage** — `toku.db` vs `sync.db`.
- **Protocol** — trusted-server recompute vs zero-knowledge SRP handshake.

So the two tiers share a **password-hardening function, not accounts**. There is no linkage, no
shared identity, and no cross-store lookup: they are two independent account systems that happen
to harden credentials the same way.

### Who actually experiences the split

The split is only observable when *both* authenticated surfaces run for the same person:

- **Default local dashboard** (`toku serve`, no `--hosted`) reads **no** web-auth tables at all;
  it is loopback-only, single-user, unauthenticated (`docs/web-auth.md:16-25`). No split.
- **Managed kafkade tier is relay-only.** ADR-014 D1 rules that a managed operator runs **only**
  the zero-knowledge relay and **must never run `toku-web`**
  (`docs/adr/014-managed-multitenant-saas.md:109-127`), precisely because the dashboard is
  trusted-server. So a managed multi-account deployment only ever touches the sync `users` store —
  there is no second store in that world.
- **Self-hoster running both** `toku serve --hosted` **and** `toku-sync` on their own box is the
  one case that sees two account stores: two admin accounts, two salts/verifiers, two lockout
  states, potentially two different passwords.

In short, the "two account stores" fact is real but its blast radius is a **single self-hoster
running both surfaces** — not the managed offering, and not the default local user.

## Decision (Option B — the two stores remain separate by design)

Toku **keeps the web and sync account stores separate**, and this ADR records **why**, rather than
unifying them onto a single backend.

**Invariant.** `srp_verifier_input()` (`crates/toku-core/src/crypto/srp.rs:40-58`) is and remains
the **only** shared authentication code between the tiers. Salts, verifiers, session tables,
storage databases, and the authentication protocol stay independent; neither store reads the
other's rows, and there is no account linkage.

**Rationale — incompatible trust models.** The two tiers sit on opposite sides of Toku's core
trust boundary and a single backend cannot serve both coherently:

- The **web dashboard must receive plaintext** — password and Secret Key — because it decrypts and
  renders the library server-side (`docs/web-auth.md:90-107`). It is trusted-server by
  construction.
- The **sync relay must never receive plaintext** — its whole value is that the operator cannot
  read user content (ADR-010; ADR-014 D1). It is zero-knowledge by construction.

Merging them would either (a) pull plaintext-handling behaviour into the zero-knowledge store,
eroding the property that makes a third-party relay acceptable, or (b) bolt a zero-knowledge SRP
handshake onto a surface that already holds plaintext — buying no confidentiality while adding a
schema migration and cross-tier coupling. Both are net-negative today. Separation is therefore the
coherent decision, not merely the status-quo one.

### Coherence / support risks acknowledged

Keeping two stores has an honest cost, borne entirely by the self-hoster who runs both surfaces:

- **Duplicate account setup.** Two first-run onboardings, two Emergency Kits, two admin accounts.
- **Divergent credentials.** The web admin password/Secret Key and the sync account
  password/Secret Key can drift apart; there is no single sign-on between them.
- **Separate lockout & session state.** A lockout or logout on one tier has no effect on the other.
- **Doubled support surface.** "I can log into sync but not the dashboard" (and vice versa) is a
  plausible support question for a two-surface self-host.

These are **usability/support** costs. They carry **no security, zero-knowledge, or
data-ownership impact**: both stores are locally owned by the operator, the relay's ZK guarantee
is untouched, and the user still owns 100% of their data in open formats.

### Non-breaking future paths (documented, not mandated)

Two additive paths could reduce the friction later, without a breaking change and without
weakening either trust model. This ADR records them and **defers the choice between them to
product intent** about whether Toku ever ships a hosted web app:

1. **Web federates to the sync account server.** `web_users` was intentionally shaped to mirror
   sync's `users` "so the web tier can later federate to a `toku-sync` account server without a
   schema rewrite" (`crates/toku-db/migrations/V16__web_auth.sql:7-11`). The web tier could
   delegate account/session management to sync's account endpoints while continuing to render
   locally — one account store, two consumers.
2. **A browser-local WASM zero-knowledge web client.** ADR-014 D1 future work notes that a web UI
   which performs the unlock and all crypto **client-side** — the server never seeing plaintext —
   is the correct long-term path to a hosted "web app"
   (`docs/adr/014-managed-multitenant-saas.md:123-127`). Such a client would authenticate against
   the zero-knowledge relay directly, dissolving the trusted-server/ZK split at its root.

Neither is required by this ADR; both remain open, additive options.

## Consequences

### Positive

- Records the trust boundary explicitly: trusted-server web and zero-knowledge sync are
  **intentionally** distinct account systems, not an accident to be "fixed" by a risky merge.
- Preserves the zero-knowledge relay guarantee (ADR-010, ADR-014) with zero code change: no
  plaintext-handling behaviour migrates into the sync store.
- Keeps Toku's non-negotiables intact — local-first/offline, no social features, user data
  ownership, frictionless import, CLI-first — all unaffected by an auth-store decision.
- Leaves two clean, additive convergence paths open for when (and if) product direction calls for
  them.

### Negative / costs

- The self-hoster running both surfaces still manages two accounts and two sets of credentials;
  this ADR accepts that cost rather than eliminating it now.
- "Auth coherence" is documented, not achieved — the ergonomic unification is deferred, so anyone
  expecting single sign-on across the dashboard and the sync server will not find it yet.
- A genuinely convergent web experience still waits on future work (federation or a WASM ZK
  client), neither of which is scheduled here.

## Alternatives Considered

- **Unify now on a single auth backend (rejected).** Superficially the "coherent" choice, but the
  two tiers have opposite plaintext requirements (web must receive it; sync must never). A single
  backend would force one model onto the other; unifying today is premature and adds migration and
  coupling risk for a problem only a dual-surface self-hoster experiences.
- **Force a zero-knowledge SRP handshake onto the web tier (rejected).** The dashboard already
  holds plaintext to render the decrypted library (`docs/web-auth.md:90-107`), so a ZK handshake
  at the login boundary would buy **no** confidentiality while adding a challenge table and
  handshake code — cost without benefit.
- **Merge the web tier into sync's `users` store (rejected for now).** Attractive because the
  schemas already match (`V16:7-11`), but it couples a trusted-server tier to the zero-knowledge
  store and makes the dashboard depend on a running `toku-sync`, which the web tier is explicitly
  designed **not** to require (`docs/web-auth.md:12-14`). Retained instead as the non-mandated
  **federation** future path above.
- **Do nothing / leave it undocumented (rejected).** The two-store split is real and would keep
  resurfacing in managed/multi-account discussions. Recording the decision and its narrow blast
  radius (this ADR) is cheaper than re-litigating it each time.

## CI / Terraform note

This ADR is **docs-only** and adds no CI job. The merge gate remains `Validate`
(`.github/workflows/validate.yml`), whose name is mirrored in the branch-protection IaC
(`kafkade/github-infra`, `repo_toku.tf`); this change renames, adds, and removes **no** gating
job, so **no Terraform change is required**. Any future *implementation* of the convergence paths
(federation or a WASM ZK client) would arrive under a later issue and own its CI/IaC coordination
at that time.

## References

Verified against the tree on this branch:

- Web auth store (trusted-server): schema `crates/toku-db/migrations/V16__web_auth.sql:7-11,13,33`;
  verifier recompute + constant-time compare `crates/toku-web/src/auth.rs:143-146,348-350`;
  trusted-server posture `crates/toku-web/src/auth.rs:13-22`, `docs/web-auth.md:12-14,16-25,90-107`.
- Sync auth store (zero-knowledge): schema
  `crates/toku-sync/migrations/V5__users_and_admin.sql:11,44,56`; full SRP-6a handshake
  `crates/toku-sync/src/auth.rs:407-635`.
- Shared seam: `srp_verifier_input()` `crates/toku-core/src/crypto/srp.rs:40-58`.
- Managed relay-only constraint (who's affected): `docs/adr/014-managed-multitenant-saas.md:109-127`;
  WASM ZK future path `docs/adr/014-managed-multitenant-saas.md:123-127`.
- Related ADRs: `docs/adr/010-self-host-auth.md`, `docs/adr/013-local-identity-key-bootstrap.md`,
  `docs/adr/014-managed-multitenant-saas.md`, `docs/adr/016-at-rest-encryption.md`.
