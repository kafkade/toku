//! Pluggable outbound email for self-serve signup verification (issue #206,
//! ADR-014 D4).
//!
//! Email is **account metadata**, not library content — the verification link
//! carries only an opaque one-time token, so this path never touches the
//! zero-knowledge data boundary. Delivery is intentionally pluggable:
//!
//! - [`LoggingMailer`] (default) logs the verification link. This keeps the
//!   self-hosted and offline relay working with no SMTP dependency, and lets
//!   tests capture the link without a real mail server.
//! - [`SmtpMailer`] delivers over SMTP (via `lettre`) and is wired only when the
//!   operator configures `--smtp-url` / `--smtp-from`.

use std::sync::{Arc, Mutex};

use crate::error::SyncError;

/// Deliver account-lifecycle emails. Implementations must be cheap to clone via
/// `Arc` and safe to share across request handlers.
pub trait Mailer: Send + Sync {
    /// Deliver an email-verification message to `to` containing `verify_url`.
    fn send_verification(&self, to: &str, verify_url: &str) -> Result<(), SyncError>;
}

/// Default mailer: logs the verification link instead of sending it. Used when
/// no SMTP relay is configured (self-hosted / dev) and by the test harness.
#[derive(Debug, Default, Clone)]
pub struct LoggingMailer;

impl Mailer for LoggingMailer {
    fn send_verification(&self, to: &str, verify_url: &str) -> Result<(), SyncError> {
        tracing::info!(
            target: "toku_sync::mailer",
            to,
            verify_url,
            "email verification link generated (no SMTP configured — logging only)"
        );
        Ok(())
    }
}

/// In-memory mailer that records every message. Test-only helper so integration
/// tests can read back the verification link a real mailer would have sent.
#[derive(Debug, Default, Clone)]
pub struct CapturingMailer {
    sent: Arc<Mutex<Vec<(String, String)>>>,
}

impl CapturingMailer {
    pub fn new() -> Self {
        Self::default()
    }

    /// All `(to, verify_url)` pairs captured so far.
    pub fn sent(&self) -> Vec<(String, String)> {
        self.sent.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// The most recently captured verification URL, if any.
    pub fn last_url(&self) -> Option<String> {
        self.sent
            .lock()
            .ok()
            .and_then(|v| v.last().map(|(_, url)| url.clone()))
    }
}

impl Mailer for CapturingMailer {
    fn send_verification(&self, to: &str, verify_url: &str) -> Result<(), SyncError> {
        if let Ok(mut sent) = self.sent.lock() {
            sent.push((to.to_string(), verify_url.to_string()));
        }
        Ok(())
    }
}

/// SMTP-backed mailer (via `lettre`), used when the operator configures an SMTP
/// relay. Built from a connection URL such as
/// `smtps://user:pass@smtp.example.com:465`.
pub struct SmtpMailer {
    transport: lettre::SmtpTransport,
    from: lettre::message::Mailbox,
}

impl SmtpMailer {
    /// Construct an SMTP mailer from a relay URL and a `From:` mailbox. Returns
    /// an error if either is malformed; delivery failures surface later, per
    /// send.
    pub fn from_config(url: &str, from: &str) -> Result<Self, SyncError> {
        let transport = lettre::SmtpTransport::from_url(url)
            .map_err(|e| SyncError::Internal(format!("invalid SMTP url: {e}")))?
            .build();
        let from = from
            .parse::<lettre::message::Mailbox>()
            .map_err(|e| SyncError::Internal(format!("invalid SMTP from address: {e}")))?;
        Ok(Self { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send_verification(&self, to: &str, verify_url: &str) -> Result<(), SyncError> {
        use lettre::Transport;
        use lettre::message::header::ContentType;

        let to_mbox = to
            .parse::<lettre::message::Mailbox>()
            .map_err(|e| SyncError::BadRequest(format!("invalid recipient address: {e}")))?;

        let email = lettre::Message::builder()
            .from(self.from.clone())
            .to(to_mbox)
            .subject("Verify your Toku sync account")
            .header(ContentType::TEXT_PLAIN)
            .body(format!(
                "Confirm your email address to activate your Toku sync account:\n\n{verify_url}\n\nIf you did not request this, you can ignore this message.\n"
            ))
            .map_err(|e| SyncError::Internal(format!("failed to build email: {e}")))?;

        self.transport
            .send(&email)
            .map_err(|e| SyncError::Internal(format!("failed to send verification email: {e}")))?;
        Ok(())
    }
}
