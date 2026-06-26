//! The Emergency Kit: the printable artifact that captures a user's account
//! details and their Secret Key.
//!
//! The Secret Key is surfaced **exactly once**, at account creation, and is
//! never recoverable from the server (see ADR-010 and `docs/recovery.md`). The
//! Emergency Kit is the user's offline record of it.
//!
//! This module is pure (no I/O): it renders the kit to plain text and to a
//! self-contained, print-friendly HTML document. Binary formats such as PDF are
//! rendered by the consuming application (the CLI) so `toku-core` stays
//! WASM/FFI-friendly and dependency-light.

use chrono::{DateTime, Utc};

/// The application label printed on the kit (e.g. on the header).
pub const EMERGENCY_KIT_APP_LABEL: &str = "Toku";

/// The standard warning printed on every Emergency Kit.
pub const EMERGENCY_KIT_WARNING: &str = "Store this somewhere safe and offline. Your Secret Key cannot be recovered. \
If you lose it and have no local copy of your library, your server data is unrecoverable. \
There is no server-side recovery and no password reset that bypasses the Secret Key.";

/// A rendered Emergency Kit.
///
/// Construct with [`EmergencyKit::new`], then render with [`EmergencyKit::to_text`]
/// or [`EmergencyKit::to_html`].
#[derive(Debug, Clone)]
pub struct EmergencyKit {
    /// The account email / identifier.
    pub account_email: String,
    /// The sync server URL, if the account is tied to a specific server.
    pub server_url: Option<String>,
    /// The formatted Secret Key (e.g. `TK-...`). See [`super::SecretKey::format`].
    pub secret_key: String,
    /// When the kit was generated.
    pub created_at: DateTime<Utc>,
}

impl EmergencyKit {
    /// Create a new Emergency Kit, stamped at the current time.
    pub fn new(
        account_email: impl Into<String>,
        server_url: Option<String>,
        secret_key: impl Into<String>,
    ) -> Self {
        Self {
            account_email: account_email.into(),
            server_url,
            secret_key: secret_key.into(),
            created_at: Utc::now(),
        }
    }

    /// Override the creation timestamp (useful for deterministic rendering/tests).
    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    fn server_display(&self) -> &str {
        self.server_url.as_deref().unwrap_or("(not set)")
    }

    fn created_display(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
    }

    /// Render the kit as plain text suitable for a terminal or `.txt` file.
    pub fn to_text(&self) -> String {
        format!(
            "\
============================================================
  {label} — EMERGENCY KIT
============================================================

  Created:   {created}

  Account:   {email}
  Server:    {server}

  Secret Key:
      {secret_key}

  Password:
      ______________________________________________
      (write your account password here, by hand)

------------------------------------------------------------
  {warning}
============================================================
",
            label = EMERGENCY_KIT_APP_LABEL,
            created = self.created_display(),
            email = self.account_email,
            server = self.server_display(),
            secret_key = self.secret_key,
            warning = EMERGENCY_KIT_WARNING,
        )
    }

    /// Render the kit as a self-contained, print-friendly HTML document.
    ///
    /// The output inlines all styling so it can be opened in any browser and
    /// printed (or "printed to PDF") without external assets.
    pub fn to_html(&self) -> String {
        format!(
            "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{label} — Emergency Kit</title>\n\
<style>\n\
  :root {{ color-scheme: light; }}\n\
  * {{ box-sizing: border-box; }}\n\
  body {{ font-family: -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;\n\
         color: #1a1a1a; background: #fff; margin: 0; padding: 2rem; }}\n\
  .kit {{ max-width: 40rem; margin: 0 auto; border: 2px solid #1a1a1a;\n\
          border-radius: 12px; padding: 2rem; }}\n\
  h1 {{ font-size: 1.5rem; margin: 0 0 0.25rem; }}\n\
  .created {{ color: #555; font-size: 0.85rem; margin-bottom: 1.5rem; }}\n\
  .row {{ margin-bottom: 1.25rem; }}\n\
  .label {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em;\n\
            color: #555; margin-bottom: 0.25rem; }}\n\
  .value {{ font-size: 1rem; }}\n\
  .secret {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;\n\
             font-size: 1.25rem; font-weight: 700; letter-spacing: 0.04em;\n\
             padding: 0.75rem 1rem; background: #f4f4f5; border-radius: 8px;\n\
             word-break: break-all; }}\n\
  .password-line {{ border-bottom: 1px solid #1a1a1a; height: 1.75rem; }}\n\
  .password-hint {{ color: #555; font-size: 0.8rem; margin-top: 0.25rem; }}\n\
  .warning {{ margin-top: 1.5rem; padding: 1rem; border: 2px solid #b00020;\n\
              border-radius: 8px; color: #b00020; font-weight: 600;\n\
              font-size: 0.9rem; line-height: 1.45; }}\n\
  @media print {{ body {{ padding: 0; }} .kit {{ border-color: #000; }} }}\n\
</style>\n\
</head>\n\
<body>\n\
  <div class=\"kit\">\n\
    <h1>{label} — Emergency Kit</h1>\n\
    <div class=\"created\">Created: {created}</div>\n\
    <div class=\"row\">\n\
      <div class=\"label\">Account</div>\n\
      <div class=\"value\">{email}</div>\n\
    </div>\n\
    <div class=\"row\">\n\
      <div class=\"label\">Server</div>\n\
      <div class=\"value\">{server}</div>\n\
    </div>\n\
    <div class=\"row\">\n\
      <div class=\"label\">Secret Key</div>\n\
      <div class=\"secret\">{secret_key}</div>\n\
    </div>\n\
    <div class=\"row\">\n\
      <div class=\"label\">Password</div>\n\
      <div class=\"password-line\"></div>\n\
      <div class=\"password-hint\">Write your account password here, by hand.</div>\n\
    </div>\n\
    <div class=\"warning\">{warning}</div>\n\
  </div>\n\
</body>\n\
</html>\n",
            label = EMERGENCY_KIT_APP_LABEL,
            created = self.created_display(),
            email = html_escape(&self.account_email),
            server = html_escape(self.server_display()),
            secret_key = html_escape(&self.secret_key),
            warning = html_escape(EMERGENCY_KIT_WARNING),
        )
    }
}

/// Minimal HTML escaping for the small set of user-supplied fields.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EmergencyKit {
        EmergencyKit::new(
            "reader@example.com",
            Some("https://toku.example.com".to_string()),
            "TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23",
        )
        .with_created_at(
            DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn text_contains_required_fields() {
        let text = sample().to_text();
        assert!(text.contains("reader@example.com"));
        assert!(text.contains("https://toku.example.com"));
        assert!(text.contains("TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23"));
        assert!(text.contains("2026-01-02 03:04 UTC"));
    }

    #[test]
    fn text_has_password_placeholder_and_warning() {
        let text = sample().to_text();
        assert!(text.to_lowercase().contains("password"));
        assert!(text.contains("write your account password"));
        assert!(text.contains("cannot be recovered"));
    }

    #[test]
    fn html_contains_required_fields_and_warning() {
        let html = sample().to_html();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("reader@example.com"));
        assert!(html.contains("TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23"));
        assert!(html.contains("cannot be recovered"));
        assert!(html.to_lowercase().contains("password"));
    }

    #[test]
    fn html_escapes_user_fields() {
        let kit = EmergencyKit::new(
            "a<b>&\"'@x.com",
            None,
            "TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23",
        );
        let html = kit.to_html();
        assert!(html.contains("a&lt;b&gt;&amp;&quot;&#39;@x.com"));
        assert!(!html.contains("a<b>&\"'@x.com"));
    }

    #[test]
    fn missing_server_renders_placeholder() {
        let kit = EmergencyKit::new("x@y.com", None, "TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23");
        assert!(kit.to_text().contains("(not set)"));
        assert!(kit.to_html().contains("(not set)"));
    }
}
