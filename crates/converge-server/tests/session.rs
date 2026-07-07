//! The browser session flow: token → cookie → authenticated requests →
//! logout (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TOKEN, server};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// A raw request with explicit headers — the harness `send` always
/// presents the bearer token, which is exactly what this suite must not.
async fn raw(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<(String, String)>, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    let body = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, headers, value)
}

fn set_cookie(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.clone())
        .expect("a Set-Cookie header")
}

#[tokio::test]
async fn session_round_trip() {
    let (_pg, _store, app) = server().await;

    // Exchange the bearer token for the session cookie.
    let (status, headers, _) = raw(
        &app,
        "POST",
        "/api/v1/session",
        &[],
        Some(serde_json::json!({ "token": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cookie = set_cookie(&headers);
    assert!(cookie.starts_with("converge_session="), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");

    // The cookie alone (no bearer) authenticates; me = the token's owner.
    let pair = cookie.split(';').next().unwrap().to_string();
    let (status, _, me) = raw(
        &app,
        "GET",
        "/api/v1/users/me",
        &[(header::COOKIE, pair.as_str())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["handle"], "admin");
    assert_eq!(me["provider"], "local");

    // Logout answers with an expired cookie.
    let (status, headers, _) = raw(&app, "DELETE", "/api/v1/session", &[], None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let cleared = set_cookie(&headers);
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

#[tokio::test]
async fn bad_credentials_stay_out() {
    let (_pg, _store, app) = server().await;

    // An unknown token buys no cookie.
    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/session",
        &[],
        Some(serde_json::json!({ "token": "cvg_wrong" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    // A forged or garbage cookie is a 401, not an error page.
    let (status, _, _) = raw(
        &app,
        "GET",
        "/api/v1/users/me",
        &[(header::COOKIE, "converge_session=not-a-jwt")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
