//! `/api/v1/tokens` — the caller-scoped token management flow
//! (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TOKEN, server};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Like the harness `send`, but with a caller-chosen bearer.
async fn send_as(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn token_lifecycle_over_rest() {
    let (_pg, _store, app) = server().await;

    // Mint: the secret comes back exactly once.
    let (status, minted) = send_as(
        &app,
        "POST",
        "/api/v1/tokens",
        TOKEN,
        Some(json!({ "label": "laptop" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let id = minted["id"].as_str().unwrap().to_owned();
    let secret = minted["token"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("cvg_"));

    // The fresh secret is a working credential.
    let (status, me) = send_as(&app, "GET", "/api/v1/users/me", &secret, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["handle"], "admin");

    // Listing shows the caller's tokens (harness token + the new one),
    // labels but never secrets.
    let (status, listed) = send_as(&app, "GET", "/api/v1/tokens", TOKEN, None).await;
    assert_eq!(status, StatusCode::OK);
    let items = listed["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|t| t["label"] == "laptop"));
    assert!(items.iter().all(|t| t.get("token").is_none()));

    // Revoke: the credential dies immediately; a second revoke is 404.
    let (status, _) = send_as(&app, "DELETE", &format!("/api/v1/tokens/{id}"), TOKEN, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send_as(&app, "GET", "/api/v1/users/me", &secret, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send_as(&app, "DELETE", &format!("/api/v1/tokens/{id}"), TOKEN, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
