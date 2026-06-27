-- Web-tier authentication for `toku serve` hosted mode (issue #122).
--
-- Additive and backward-compatible: local single-user mode (the historical
-- default) never reads these tables. They are only consulted when the dashboard
-- runs in hosted mode, where unauthenticated requests are redirected to login.
--
-- These mirror the shape of toku-sync's `users` / `user_sessions` (migration
-- V5 in that crate) so the web tier can later federate to a toku-sync account
-- server without a schema rewrite. The web tier stores only the SRP verifier +
-- salt (never the password or Secret Key) and opaque wrapped key material; it
-- cannot derive any plaintext secret from these columns.

CREATE TABLE web_users (
    id                   TEXT    PRIMARY KEY,            -- uuid v7
    email                TEXT    NOT NULL UNIQUE,        -- account handle / login
    srp_salt             TEXT    NOT NULL,               -- hex-encoded SRP salt
    srp_verifier         TEXT    NOT NULL,               -- hex-encoded SRP verifier (g^x mod N)
    wrapped_private_key  TEXT,                           -- opaque wrapped account private key (#116)
    account_public_key   TEXT,                           -- account public key (#116)
    kdf_params           TEXT,                           -- versioned KDF params blob (#116)
    role                 TEXT    NOT NULL DEFAULT 'user'
                                 CHECK (role IN ('admin', 'user')),
    status               TEXT    NOT NULL DEFAULT 'active'
                                 CHECK (status IN ('active', 'disabled')),
    failed_attempts      INTEGER NOT NULL DEFAULT 0,
    locked_until         TEXT,                           -- ISO-8601 datetime; NULL = not locked
    created_at           TEXT    NOT NULL
);

-- Cookie-backed browser sessions issued after a successful login. The raw
-- session token lives only in the user's cookie; the server stores its SHA-256
-- hash. CSRF is enforced separately via a double-submit cookie.
CREATE TABLE web_sessions (
    token_hash   TEXT PRIMARY KEY,   -- sha256(session token)
    user_id      TEXT NOT NULL REFERENCES web_users(id),
    expires_at   TEXT NOT NULL,      -- ISO-8601 UTC datetime
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_web_sessions_expires ON web_sessions(expires_at);
