use clap::Parser;

/// Toku sync relay server — stores and relays sync operations between devices.
#[derive(Debug, Parser)]
#[command(name = "toku-sync", version)]
pub struct Config {
    /// Port to listen on
    #[arg(long, default_value = "8080", env = "TOKU_SYNC_PORT")]
    pub port: u16,

    /// Address to bind to
    #[arg(long, default_value = "0.0.0.0", env = "TOKU_SYNC_BIND")]
    pub bind: String,

    /// Directory for server data (SQLite database)
    #[arg(long, default_value = "./toku-sync-data", env = "TOKU_SYNC_DATA_DIR")]
    pub data_dir: String,

    /// Log verbosity (error, warn, info, debug, trace). Overridden by `RUST_LOG` if set.
    #[arg(long, default_value = "info", env = "TOKU_SYNC_LOG_LEVEL")]
    pub log_level: String,

    // ── Managed-tier controls (issue #206, ADR-014) ──────────────────────────
    //
    // Every knob below defaults to the self-hosted behaviour: no quota, no
    // per-user rate limit, no email verification. A self-hosted or offline relay
    // is unchanged until an operator opts in. None of these affect the
    // zero-knowledge guarantee — the server still only ever holds ciphertext.
    /// Per-account stored-ciphertext ceiling in bytes. Unset = unlimited
    /// (self-hosted default). Applies to every owned account unless overridden
    /// per user via the `user_quota` entitlement table.
    #[arg(long, env = "TOKU_SYNC_DEFAULT_MAX_USER_BYTES")]
    pub default_max_user_bytes: Option<i64>,

    /// Per-account stored-op ceiling. Unset = unlimited (self-hosted default).
    #[arg(long, env = "TOKU_SYNC_DEFAULT_MAX_USER_OPS")]
    pub default_max_user_ops: Option<i64>,

    /// Per-authenticated-user request ceiling within the rate window. 0 =
    /// disabled (self-hosted default); the per-IP + global limiter still applies.
    #[arg(long, default_value = "0", env = "TOKU_SYNC_PER_USER_RATE_MAX")]
    pub per_user_rate_max: u32,

    /// Fixed-window length (seconds) for the per-user rate limiter.
    #[arg(
        long,
        default_value = "60",
        env = "TOKU_SYNC_PER_USER_RATE_WINDOW_SECS"
    )]
    pub per_user_rate_window_secs: u64,

    /// Require new (non-admin) signups to confirm their email before they can
    /// obtain a session. Off by default (self-hosted). When on, `--smtp-url`,
    /// `--smtp-from`, and `--public-base-url` must also be set.
    #[arg(
        long,
        default_value = "false",
        env = "TOKU_SYNC_REQUIRE_EMAIL_VERIFICATION"
    )]
    pub require_email_verification: bool,

    /// SMTP relay URL for verification email delivery, e.g.
    /// `smtps://user:pass@smtp.example.com:465`. Unset = log the link instead of
    /// sending (dev / self-host).
    #[arg(long, env = "TOKU_SYNC_SMTP_URL")]
    pub smtp_url: Option<String>,

    /// `From:` address for verification emails, e.g. `Toku <no-reply@example.com>`.
    #[arg(long, env = "TOKU_SYNC_SMTP_FROM")]
    pub smtp_from: Option<String>,

    /// Public base URL the server is reachable at, used to build verification
    /// links, e.g. `https://sync.example.com`.
    #[arg(long, env = "TOKU_SYNC_PUBLIC_BASE_URL")]
    pub public_base_url: Option<String>,
}

impl Config {
    pub fn db_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.data_dir).join("sync.db")
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}
