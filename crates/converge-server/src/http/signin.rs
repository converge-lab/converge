//! The browser's IdP sign-in: `/auth/login` → provider → `/auth/callback`,
//! plus the open capability read (`GET /api/v1/auth`) the login screen
//! uses to decide whether to offer the button.
//!
//! All three are open paths — they are the gate's entrance. The
//! per-attempt secrets (CSRF `state`, PKCE verifier) ride an `HttpOnly`
//! cookie between the two ends of the redirect; `SameSite=Lax` because the
//! callback arrives as a top-level navigation *from the provider* —
//! `Strict` would withhold the cookie exactly then. Callback failures
//! answer plain text: there is no session yet to decorate an app screen
//! with, and the operator needs the reason verbatim.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use converge_storage::{AuthInfo, Storage};
use serde::Deserialize;

use crate::auth::{self, Sessions};
use crate::oidc::Oidc;

/// The flow cookie: state + verifier for the round trip to the provider.
const FLOW: &str = "converge_oauth";

pub fn routes<S: Storage + 'static>() -> Router<(S, Sessions, Arc<Option<Oidc>>)> {
    Router::new()
        .route("/api/v1/auth", get(info::<S>))
        .route("/auth/login", get(login::<S>))
        .route("/auth/callback", get(callback::<S>))
}

async fn info<S: Storage>(
    State((_, _, oidc)): State<(S, Sessions, Arc<Option<Oidc>>)>,
) -> Json<AuthInfo> {
    Json(AuthInfo {
        oidc: oidc.as_ref().as_ref().map(Oidc::label),
    })
}

/// Where to land after a successful sign-in. Only same-origin paths —
/// anything else (absolute URLs, protocol-relative `//`) is an open
/// redirect and collapses to `/`.
fn landing(next: Option<&str>) -> &str {
    match next {
        Some(next) if next.starts_with('/') && !next.starts_with("//") => next,
        _ => "/",
    }
}

#[derive(Deserialize)]
struct Login {
    /// Same-origin path to resume after sign-in (e.g. an OAuth authorize
    /// URL for an MCP connector).
    #[serde(default)]
    next: Option<String>,
}

async fn login<S: Storage>(
    State((_, _, oidc)): State<(S, Sessions, Arc<Option<Oidc>>)>,
    jar: CookieJar,
    Query(login): Query<Login>,
) -> Response {
    let Some(oidc) = oidc.as_ref() else {
        return (StatusCode::NOT_FOUND, "no sign-in provider is configured").into_response();
    };
    match oidc.authorize().await {
        Ok((url, flow)) => {
            let next = landing(login.next.as_deref());
            // `next` rides third; it may itself contain dots, so parsing
            // splits from the left exactly twice.
            let cookie = Cookie::build((FLOW, format!("{}.{}.{next}", flow.state, flow.verifier)))
                .http_only(true)
                .same_site(SameSite::Lax)
                .path("/auth")
                .max_age(time::Duration::minutes(10))
                .build();
            (jar.add(cookie), Redirect::to(&url)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "sign-in could not start");
            (
                StatusCode::BAD_GATEWAY,
                format!("sign-in could not start: {e}"),
            )
                .into_response()
        }
    }
}

/// What the provider sends back — a code, or an error pair.
#[derive(Deserialize)]
struct Callback {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

async fn callback<S: Storage>(
    State((store, sessions, oidc)): State<(S, Sessions, Arc<Option<Oidc>>)>,
    jar: CookieJar,
    Query(cb): Query<Callback>,
) -> Response {
    let Some(oidc) = oidc.as_ref() else {
        return (StatusCode::NOT_FOUND, "no sign-in provider is configured").into_response();
    };
    if let Some(error) = cb.error {
        let detail = cb.error_description.unwrap_or_default();
        return (
            StatusCode::BAD_REQUEST,
            format!("the provider declined the sign-in: {error} {detail}"),
        )
            .into_response();
    }
    let (Some(code), Some(state)) = (cb.code, cb.state) else {
        return (StatusCode::BAD_REQUEST, "missing code or state").into_response();
    };
    // Double-submit check: the state must match the one this browser was
    // handed at /auth/login; the PKCE verifier travels with it.
    let Some(flow) = jar.get(FLOW).map(|c| c.value().to_string()) else {
        return (
            StatusCode::BAD_REQUEST,
            "no sign-in in progress (the flow cookie is missing or expired; start over)",
        )
            .into_response();
    };
    let mut parts = flow.splitn(3, '.');
    let (Some(expected), Some(verifier)) = (parts.next(), parts.next()) else {
        return (StatusCode::BAD_REQUEST, "malformed flow cookie; start over").into_response();
    };
    let next = landing(parts.next()).to_string();
    if expected != state {
        return (StatusCode::BAD_REQUEST, "state mismatch; start over").into_response();
    }

    let identity = match oidc.exchange(&code, verifier).await {
        Ok(identity) => identity,
        Err(e) => {
            tracing::error!(error = %e, "sign-in failed at the provider");
            return (StatusCode::BAD_GATEWAY, format!("sign-in failed: {e}")).into_response();
        }
    };
    if !oidc.allowed(&identity.handle) {
        return (
            StatusCode::FORBIDDEN,
            format!(
                "`{}` is not on this deployment's allowlist",
                identity.handle
            ),
        )
            .into_response();
    }
    let user = match store.user_login(identity).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(error = %e, "sign-in failed at storage");
            return (StatusCode::SERVICE_UNAVAILABLE, "storage unavailable").into_response();
        }
    };

    // One Set-Cookie mints the session, the other retires the flow.
    let done = Cookie::build((FLOW, ""))
        .path("/auth")
        .max_age(time::Duration::ZERO);
    let jar = jar
        .add(auth::cookie(sessions.issue(user)))
        .add(done.build());
    (jar, Redirect::to(&next)).into_response()
}
