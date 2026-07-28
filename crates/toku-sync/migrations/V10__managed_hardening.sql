-- Managed-tier server hardening (issue #206, ADR-014 D2/D3/D4/D6).
--
-- Additive, idempotent, and backward-compatible: every column and table added
-- here defaults to the pre-existing self-hosted behaviour (no quota, no
-- per-user rate limit, no email verification). A self-hosted or offline relay
-- is byte-for-byte unchanged until an operator opts in via config or the
-- per-user entitlement seam below. None of these columns hold user content —
-- the zero-knowledge guarantee is untouched.

-- ── D4: signup email verification ────────────────────────────────────────────

-- Whether the account has proven control of its email. Defaults to 1 (verified)
-- so every pre-existing account, the first-run admin bootstrap, and any
-- self-hosted signup keep working without a verification round-trip. Only a
-- managed instance that turns on `require_email_verification` mints new
-- non-admin accounts as unverified (0).
ALTER TABLE users ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 1;

-- Instance-wide toggle. 0 = self-hosted default (no verification). When 1, new
-- non-admin signups must confirm their email before they can obtain a session.
ALTER TABLE instance_config ADD COLUMN require_email_verification INTEGER NOT NULL DEFAULT 0;

-- Single-use, expiring email-verification tokens. Only the SHA-256 hash of the
-- token is stored (like session tokens); the raw token travels to the user by
-- email and is never persisted in the clear. This is account metadata, not
-- library data.
CREATE TABLE email_verification_tokens (
    token_hash  TEXT PRIMARY KEY,                         -- sha256(raw token), hex
    user_id     TEXT NOT NULL REFERENCES users(id),
    expires_at  TEXT NOT NULL,                            -- ISO-8601 UTC datetime
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_email_verif_tokens_user ON email_verification_tokens(user_id);
CREATE INDEX idx_email_verif_tokens_expires ON email_verification_tokens(expires_at);

-- ── D2: per-user quotas ──────────────────────────────────────────────────────

-- Instance-wide default ceilings applied to every owned account. NULL =
-- unlimited (self-hosted default). A managed operator sets these to a positive
-- byte / op count to enforce a baseline ceiling.
ALTER TABLE instance_config ADD COLUMN default_max_user_bytes INTEGER;  -- NULL = unlimited
ALTER TABLE instance_config ADD COLUMN default_max_user_ops   INTEGER;  -- NULL = unlimited

-- Per-user entitlement override (ADR-014 D5 plan-lookup seam, cached
-- server-side). This is the structural input the D2 quota check reads to
-- resolve a ceiling for a given account; it carries only capability ceilings
-- (bytes / ops), never any price, tier, or billing state. A NULL column falls
-- back to the instance default (and NULL there means unlimited). An external
-- billing/plan system, if one ever exists, would write rows here — this repo
-- ships only the seam.
CREATE TABLE user_quota (
    user_id     TEXT PRIMARY KEY REFERENCES users(id),
    max_bytes   INTEGER,                                  -- NULL = fall back to instance default
    max_ops     INTEGER,                                  -- NULL = fall back to instance default
    updated_at  TEXT NOT NULL
);
