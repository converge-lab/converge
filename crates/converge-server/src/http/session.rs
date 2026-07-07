//! `/api/v1/session` — the browser's credential exchange (a singleton
//! resource: you hold at most one).
//!
//! `POST` swaps a bearer token for the `HttpOnly` session cookie, so the
//! pasted secret never persists in the browser; `DELETE` clears the
//! cookie (logout). Both are *open* paths — they are the gate's entrance
//! — and both answer 204 with the cookie change riding `Set-Cookie`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use converge_storage::Storage;
use serde::Deserialize;
use serde_json::json;

use super::error::Error;
use crate::auth::{self, COOKIE, SESSION_TTL, Sessions};

pub fn routes<S: Storage + 'static>() -> Router<(S, Sessions)> {
    Router::new().route("/api/v1/session", post(login::<S>).delete(logout))
}

#[derive(Deserialize)]
struct Login {
    /// A bearer token secret (`cvg_…`), e.g. from `converge-server token
    /// mint`.
    token: String,
}

async fn login<S: Storage>(
    State((store, sessions)): State<(S, Sessions)>,
    jar: CookieJar,
    Json(login): Json<Login>,
) -> Response {
    let user = match store.token_user(&auth::hash(&login.token)).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "code": "unauthorized", "message": "unknown token" } })),
            )
                .into_response();
        }
        Err(e) => return Error::from(e).into_response(),
    };
    let cookie = Cookie::build((COOKIE, sessions.issue(user)))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(SESSION_TTL)
        .build();
    (jar.add(cookie), StatusCode::NO_CONTENT).into_response()
}

async fn logout(jar: CookieJar) -> (CookieJar, StatusCode) {
    // An explicit expired cookie, not `jar.remove` — remove only answers
    // when the request carried the original, and logout must clear the
    // browser unconditionally. Path must match the set cookie's.
    let cookie = Cookie::build((COOKIE, ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .build();
    (jar.add(cookie), StatusCode::NO_CONTENT)
}
