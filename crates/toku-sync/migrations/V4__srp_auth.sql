-- SRP-6a authentication: per-library account holding the SRP verifier and salt.
-- The server only ever stores the verifier (g^x mod N) — never the password.
-- Username used in SRP computations is the library_id.
CREATE TABLE accounts (
    library_id        TEXT    PRIMARY KEY REFERENCES libraries(id),
    srp_salt          TEXT    NOT NULL,      -- hex-encoded 16-byte random salt
    srp_verifier      TEXT    NOT NULL,      -- hex-encoded SRP verifier (≤256 bytes)
    failed_attempts   INTEGER NOT NULL DEFAULT 0,
    locked_until      TEXT    -- ISO-8601 datetime; NULL means not locked
);

-- Ephemeral SRP server challenges (single-use, expire after 5 minutes).
-- server_ephemeral_secret = hex-encoded b (48-byte random value).
-- client_public_a is stored here so the verify handler can reconstruct M1.
CREATE TABLE srp_challenges (
    challenge_id             TEXT PRIMARY KEY,
    library_id               TEXT NOT NULL REFERENCES libraries(id),
    server_ephemeral_secret  TEXT NOT NULL,  -- hex-encoded b
    client_public_a          TEXT NOT NULL,  -- hex-encoded A (needed for verify)
    created_at               TEXT NOT NULL
);
CREATE INDEX idx_srp_challenges_created ON srp_challenges(created_at);

-- Short-lived session tokens issued after successful SRP verification (TTL = 24 h).
-- auth_token_hash in devices is kept for backward-compatible passwordless libraries.
CREATE TABLE sessions (
    session_token_hash  TEXT    PRIMARY KEY,
    device_id           TEXT    NOT NULL REFERENCES devices(device_id),
    library_id          TEXT    NOT NULL REFERENCES libraries(id),
    expires_at          TEXT    NOT NULL,   -- ISO-8601 UTC datetime
    created_at          TEXT    NOT NULL
);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);
