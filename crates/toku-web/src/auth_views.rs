//! Views for hosted-mode authentication: login, first-run setup, and the
//! one-time Emergency Kit.

use maud::{DOCTYPE, Markup, PreEscaped, html};

const AUTH_CSS: &str = r#"
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    margin: 0; min-height: 100vh; display: flex; align-items: center;
    justify-content: center; background: #f5f5f7; color: #1d1d1f; padding: 2rem;
}
.auth-card {
    background: #fff; border-radius: 14px; box-shadow: 0 8px 30px rgba(0,0,0,.08);
    padding: 2.5rem; width: 100%; max-width: 28rem;
}
.auth-card.wide { max-width: 40rem; }
.logo { font-size: 1.5rem; font-weight: 700; margin-bottom: .25rem; }
h1 { font-size: 1.35rem; margin: .25rem 0 1.25rem; }
label { display: block; font-weight: 600; margin: 1rem 0 .35rem; font-size: .9rem; }
input[type=email], input[type=password], input[type=text] {
    width: 100%; padding: .65rem .75rem; border: 1px solid #d2d2d7;
    border-radius: 8px; font-size: 1rem;
}
.btn {
    margin-top: 1.5rem; width: 100%; padding: .7rem; border: 0; border-radius: 8px;
    background: #0071e3; color: #fff; font-size: 1rem; font-weight: 600; cursor: pointer;
}
.btn:hover { background: #0077ed; }
.muted { color: #6e6e73; font-size: .85rem; }
.error { background: #fdecea; color: #b3261e; padding: .75rem 1rem; border-radius: 8px;
    margin-bottom: 1rem; font-size: .9rem; }
.kit { background: #1d1d1f; color: #f5f5f7; padding: 1.25rem; border-radius: 10px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 1.05rem;
    letter-spacing: .04em; word-break: break-all; margin: 1rem 0; text-align: center; }
.warn { background: #fff4e5; color: #7a4f01; padding: 1rem; border-radius: 8px;
    font-size: .9rem; margin: 1rem 0; }
.actions { display: flex; gap: .75rem; margin-top: 1.5rem; }
.actions a, .actions button { flex: 1; text-align: center; }
a.secondary { display: block; padding: .7rem; border-radius: 8px; border: 1px solid #d2d2d7;
    color: #0071e3; text-decoration: none; font-weight: 600; }
"#;

fn shell(title: &str, inner: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " — Toku" }
                style { (PreEscaped(AUTH_CSS)) }
            }
            body { (inner) }
        }
    }
}

/// A hidden CSRF field, rendered only when a token is present (hosted mode).
pub fn csrf_field(token: &str) -> Markup {
    html! {
        @if !token.is_empty() {
            input type="hidden" name="csrf_token" value=(token);
        }
    }
}

/// Append `?csrf=<token>` to a form action for multipart submissions, where the
/// token can't ride in the buffered body. Returns the bare action in local mode.
pub fn action_with_csrf(action: &str, token: &str) -> String {
    if token.is_empty() {
        action.to_string()
    } else {
        format!("{action}?csrf={}", urlencoding::encode(token))
    }
}

/// The login page. `error` shows a message above the form when present.
pub fn login_page(csrf: &str, error: Option<&str>) -> Markup {
    shell(
        "Sign in",
        html! {
            div.auth-card {
                div.logo { "📚 Toku" }
                h1 { "Sign in" }
                @if let Some(msg) = error {
                    div.error { (msg) }
                }
                form method="post" action="/login" {
                    (csrf_field(csrf))
                    label for="email" { "Email" }
                    input id="email" type="email" name="email" required autofocus;
                    label for="password" { "Password" }
                    input id="password" type="password" name="password" required;
                    label for="secret_key" { "Secret Key" }
                    input id="secret_key" type="text" name="secret_key"
                          autocomplete="off" autocapitalize="off" spellcheck="false"
                          placeholder="TK-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX-XX" required;
                    p.muted { "From your Emergency Kit, shown once at setup." }
                    button.btn type="submit" { "Sign in" }
                }
            }
        },
    )
}

/// The first-run setup page (no admin exists yet).
pub fn setup_page(csrf: &str, error: Option<&str>) -> Markup {
    shell(
        "Welcome to Toku",
        html! {
            div.auth-card {
                div.logo { "📚 Toku" }
                h1 { "Create your administrator account" }
                p.muted {
                    "This is the first run of your Toku server. Create the admin \
                     account that controls this instance."
                }
                @if let Some(msg) = error {
                    div.error { (msg) }
                }
                form method="post" action="/setup" {
                    (csrf_field(csrf))
                    label for="email" { "Email" }
                    input id="email" type="email" name="email" required autofocus;
                    label for="password" { "Password" }
                    input id="password" type="password" name="password"
                          minlength="8" required;
                    p.muted { "At least 8 characters." }
                    button.btn type="submit" { "Create account" }
                }
            }
        },
    )
}

/// The one-time Emergency Kit shown immediately after account creation.
pub fn emergency_kit_page(email: &str, secret_key: &str) -> Markup {
    shell(
        "Your Emergency Kit",
        html! {
            div.auth-card.wide {
                div.logo { "📚 Toku" }
                h1 { "Save your Emergency Kit" }
                p { "Account: " strong { (email) } }
                p {
                    "This is your " strong { "Secret Key" }
                    ". It is shown " strong { "once" } " and is required — together with \
                     your password — to unlock your account on a new device."
                }
                div.kit { (secret_key) }
                div.warn {
                    "⚠️ We cannot recover this for you. There is no server-side copy. \
                     Store it offline (print it or save it in a password manager). \
                     Your local library remains your ultimate backup."
                }
                div.actions {
                    a.secondary href="/login" { "I’ve saved it — continue to sign in" }
                }
            }
        },
    )
}
