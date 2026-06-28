//! Integration tests for hosted-mode onboarding + session auth (issue #122).
//!
//! These exercise the full Axum router via `tower::ServiceExt::oneshot`, so they
//! cover the middleware stack (CSRF + auth gate) end to end.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use toku_web::{WebMode, build_router};

/// A migrated, empty database plus a temp dir, kept alive for the test.
struct TestEnv {
    _dir: tempfile::TempDir,
    db_path: std::path::PathBuf,
    temp_dir: std::path::PathBuf,
}

fn setup_env() -> TestEnv {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("toku.db");
    // build_router does NOT migrate; the real server does this once at startup.
    toku_db::Database::open(&db_path).expect("migrate db");
    let temp_dir = dir.path().join("uploads");
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    TestEnv {
        _dir: dir,
        db_path,
        temp_dir,
    }
}

fn hosted_router(env: &TestEnv) -> Router {
    // secure_cookies = false so cookies are usable over the plain-HTTP test client.
    build_router(
        env.db_path.clone(),
        env.temp_dir.clone(),
        WebMode::Hosted,
        false,
    )
}

fn local_router(env: &TestEnv) -> Router {
    build_router(
        env.db_path.clone(),
        env.temp_dir.clone(),
        WebMode::Local,
        false,
    )
}

/// Collect all `Set-Cookie` values from a response.
fn set_cookies(resp: &axum::response::Response) -> Vec<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .collect()
}

/// Extract a single cookie's value (the `name=value` part) by cookie name.
fn cookie_value(resp: &axum::response::Response, name: &str) -> Option<String> {
    set_cookies(resp).into_iter().find_map(|c| {
        let pair = c.split(';').next().unwrap_or("").trim().to_string();
        let (k, v) = pair.split_once('=')?;
        if k == name { Some(v.to_string()) } else { None }
    })
}

fn location(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap()
}

fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("GET")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn form_post(uri: &str, cookie: Option<&str>, body: String) -> Request<Body> {
    let mut b = Request::builder()
        .uri(uri)
        .method("POST")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(body)).unwrap()
}

/// Perform a GET to obtain a fresh CSRF cookie + token (they share the value in
/// the double-submit scheme), returning `(csrf_token, cookie_header)`.
async fn fetch_csrf(router: &Router, uri: &str) -> (String, String) {
    let resp = router.clone().oneshot(get(uri)).await.unwrap();
    let token = cookie_value(&resp, "toku_csrf").expect("csrf cookie issued");
    let cookie_header = format!("toku_csrf={token}");
    (token, cookie_header)
}

/// Create the admin account through the real `/setup` POST flow.
async fn create_admin(router: &Router, email: &str, password: &str) {
    let (token, cookie) = fetch_csrf(router, "/setup").await;
    let body = format!(
        "email={}&password={}&csrf_token={}",
        urlencoding::encode(email),
        urlencoding::encode(password),
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/setup", Some(&cookie), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "setup should succeed");
    let html = body_string(resp).await;
    assert!(
        html.contains("TK-"),
        "emergency kit page should show the Secret Key"
    );
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn hosted_first_run_redirects_to_setup() {
    let env = setup_env();
    let router = hosted_router(&env);

    let resp = router.clone().oneshot(get("/")).await.unwrap();
    assert!(resp.status().is_redirection());
    assert_eq!(location(&resp).as_deref(), Some("/setup"));
}

#[tokio::test]
async fn hosted_unauthenticated_redirects_to_login_once_admin_exists() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    let resp = router.clone().oneshot(get("/library")).await.unwrap();
    assert!(resp.status().is_redirection());
    assert_eq!(location(&resp).as_deref(), Some("/login"));
}

#[tokio::test]
async fn setup_rejects_second_admin() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    // GET /setup now bounces to /login.
    let resp = router.clone().oneshot(get("/setup")).await.unwrap();
    assert!(resp.status().is_redirection());
    assert_eq!(location(&resp).as_deref(), Some("/login"));

    // POST /setup is refused even with a valid CSRF token.
    let (token, cookie) = fetch_csrf(&router, "/login").await;
    let body = format!(
        "email=second%40example.com&password=another+password&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/setup", Some(&cookie), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(
        html.contains("already exists"),
        "second setup should report an existing admin, got: {html}"
    );
}

#[tokio::test]
async fn valid_login_sets_session_and_grants_access() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    let (token, cookie) = fetch_csrf(&router, "/login").await;
    let body = format!(
        "email=admin%40example.com&password=correct+horse&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();
    assert!(resp.status().is_redirection());
    assert_eq!(location(&resp).as_deref(), Some("/library"));

    let session = cookie_value(&resp, "toku_session").expect("session cookie set on login");
    assert!(!session.is_empty());

    // The session cookie now grants access to a gated route.
    let resp = router
        .clone()
        .oneshot(get_with_cookie(
            "/library",
            &format!("toku_session={session}"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn invalid_password_is_rejected() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    let (token, cookie) = fetch_csrf(&router, "/login").await;
    let body = format!(
        "email=admin%40example.com&password=wrong+password&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(cookie_value(&resp, "toku_session").is_none());
    let html = body_string(resp).await;
    assert!(html.contains("Invalid email or password"));
}

#[tokio::test]
async fn session_fixation_token_changes_on_login() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    // Attacker-supplied session cookie value that is not a valid session.
    let planted = "attacker-fixed-token";

    let (token, csrf_cookie) = fetch_csrf(&router, "/login").await;
    // Send both the CSRF cookie and the planted session cookie.
    let cookie = format!("{csrf_cookie}; toku_session={planted}");
    let body = format!(
        "email=admin%40example.com&password=correct+horse&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();

    let issued = cookie_value(&resp, "toku_session").expect("fresh session issued");
    assert_ne!(
        issued, planted,
        "login must mint a brand-new token, never reuse a pre-auth one"
    );
}

#[tokio::test]
async fn csrf_missing_token_is_forbidden() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    // POST with no CSRF cookie/token at all.
    let body = "email=admin%40example.com&password=correct+horse".to_string();
    let resp = router
        .clone()
        .oneshot(form_post("/login", None, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_bad_token_is_forbidden() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    let (_token, cookie) = fetch_csrf(&router, "/login").await;
    // Submit a token that does not match the cookie.
    let body = "email=admin%40example.com&password=correct+horse&csrf_token=not-the-real-token"
        .to_string();
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lockout_after_repeated_failures() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    // Five wrong attempts trip the lockout threshold.
    for _ in 0..5 {
        let (token, cookie) = fetch_csrf(&router, "/login").await;
        let body = format!(
            "email=admin%40example.com&password=wrong&csrf_token={}",
            urlencoding::encode(&token),
        );
        let _ = router
            .clone()
            .oneshot(form_post("/login", Some(&cookie), body))
            .await
            .unwrap();
    }

    // Even the correct password is now refused with the lockout message.
    let (token, cookie) = fetch_csrf(&router, "/login").await;
    let body = format!(
        "email=admin%40example.com&password=correct+horse&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(cookie_value(&resp, "toku_session").is_none());
    let html = body_string(resp).await;
    assert!(
        html.contains("Too many failed attempts"),
        "expected lockout message, got: {html}"
    );
}

#[tokio::test]
async fn logout_clears_session() {
    let env = setup_env();
    let router = hosted_router(&env);
    create_admin(&router, "admin@example.com", "correct horse").await;

    let (token, cookie) = fetch_csrf(&router, "/login").await;
    let body = format!(
        "email=admin%40example.com&password=correct+horse&csrf_token={}",
        urlencoding::encode(&token),
    );
    let resp = router
        .clone()
        .oneshot(form_post("/login", Some(&cookie), body))
        .await
        .unwrap();
    let session = cookie_value(&resp, "toku_session").expect("session cookie");

    // Logout invalidates the session server-side.
    let resp = router
        .clone()
        .oneshot(get_with_cookie(
            "/logout",
            &format!("toku_session={session}"),
        ))
        .await
        .unwrap();
    assert!(resp.status().is_redirection());

    // The old token no longer grants access.
    let resp = router
        .clone()
        .oneshot(get_with_cookie(
            "/library",
            &format!("toku_session={session}"),
        ))
        .await
        .unwrap();
    assert!(resp.status().is_redirection());
    assert_eq!(location(&resp).as_deref(), Some("/login"));
}

#[tokio::test]
async fn healthz_is_public_in_hosted_mode() {
    let env = setup_env();
    let router = hosted_router(&env);
    let resp = router.clone().oneshot(get("/healthz")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Local mode keeps the historical no-auth behaviour.

#[tokio::test]
async fn local_mode_requires_no_auth() {
    let env = setup_env();
    let router = local_router(&env);

    let resp = router.clone().oneshot(get("/library")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // No auth cookies are ever issued in local mode.
    let resp = router.clone().oneshot(get("/")).await.unwrap();
    assert!(cookie_value(&resp, "toku_csrf").is_none());
    assert!(cookie_value(&resp, "toku_session").is_none());
}

#[tokio::test]
async fn local_mode_has_no_login_route() {
    let env = setup_env();
    let router = local_router(&env);
    // /login is only mounted in hosted mode.
    let resp = router.clone().oneshot(get("/login")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
