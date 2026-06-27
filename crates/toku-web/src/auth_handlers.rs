//! Route handlers for hosted-mode authentication.

use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;

use crate::AppState;
use crate::auth::{self, CsrfToken};
use crate::auth_views;

#[derive(serde::Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

#[derive(serde::Deserialize)]
pub struct SetupForm {
    pub email: String,
    pub password: String,
}

/// `GET /login` — show the sign-in form (or bounce to setup when no admin).
pub async fn login_page(State(state): State<AppState>, csrf: CsrfToken) -> Response {
    let db_path = state.db_path.clone();
    match tokio::task::spawn_blocking(move || auth::admin_exists(&db_path)).await {
        Ok(Ok(false)) => return Redirect::to("/setup").into_response(),
        Ok(Ok(true)) => {}
        _ => return internal_error(),
    }
    Html(auth_views::login_page(csrf.value(), None).into_string()).into_response()
}

/// `POST /login` — verify credentials and start a session.
pub async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    csrf: CsrfToken,
    form: axum::extract::Form<LoginForm>,
) -> Response {
    let db_path = state.db_path.clone();
    let email = form.email.clone();
    let password = form.password.clone();

    let outcome =
        tokio::task::spawn_blocking(move || auth::login(&db_path, &email, &password)).await;

    match outcome {
        Ok(Ok(auth::LoginOutcome::Success { session_token })) => {
            let jar = jar.add(auth::session_cookie(session_token, state.secure_cookies));
            (jar, Redirect::to("/library")).into_response()
        }
        Ok(Ok(auth::LoginOutcome::Invalid)) => Html(
            auth_views::login_page(csrf.value(), Some("Invalid email or password.")).into_string(),
        )
        .into_response(),
        Ok(Ok(auth::LoginOutcome::Locked)) => Html(
            auth_views::login_page(
                csrf.value(),
                Some("Too many failed attempts. Try again later."),
            )
            .into_string(),
        )
        .into_response(),
        _ => internal_error(),
    }
}

/// `POST /logout` — destroy the session and clear the cookie.
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(token) = jar.get(auth::SESSION_COOKIE).map(|c| c.value().to_string()) {
        let db_path = state.db_path.clone();
        let _ = tokio::task::spawn_blocking(move || auth::delete_session(&db_path, &token)).await;
    }
    let jar = jar.add(auth::clear_session_cookie(state.secure_cookies));
    (jar, Redirect::to("/login")).into_response()
}

/// `GET /setup` — first-run onboarding (404s into login once an admin exists).
pub async fn setup_page(State(state): State<AppState>, csrf: CsrfToken) -> Response {
    let db_path = state.db_path.clone();
    match tokio::task::spawn_blocking(move || auth::admin_exists(&db_path)).await {
        Ok(Ok(true)) => return Redirect::to("/login").into_response(),
        Ok(Ok(false)) => {}
        _ => return internal_error(),
    }
    Html(auth_views::setup_page(csrf.value(), None).into_string()).into_response()
}

/// `POST /setup` — create the admin account and show the Emergency Kit once.
pub async fn setup_submit(
    State(state): State<AppState>,
    csrf: CsrfToken,
    form: axum::extract::Form<SetupForm>,
) -> Response {
    let db_path = state.db_path.clone();
    let email = form.email.clone();
    let password = form.password.clone();

    let result =
        tokio::task::spawn_blocking(move || auth::create_admin(&db_path, &email, &password)).await;

    match result {
        Ok(Ok(created)) => {
            Html(auth_views::emergency_kit_page(&created.email, &created.secret_key).into_string())
                .into_response()
        }
        Ok(Err(msg)) => {
            Html(auth_views::setup_page(csrf.value(), Some(&msg)).into_string()).into_response()
        }
        _ => internal_error(),
    }
}

/// Unauthenticated liveness probe for Docker/orchestration (#124).
pub async fn healthz() -> &'static str {
    "ok"
}

fn internal_error() -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "internal error",
    )
        .into_response()
}
