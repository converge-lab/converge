//! The device-grant pairing path (RFC 8628): discovery, device-only
//! registration, the approval API behind a browser session, polling the
//! token endpoint, and the granted credential against the API
//! (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TOKEN, server};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// A session cookie for the harness admin — the "signed-in browser".
async fn session(app: &Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": TOKEN }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// Register a device-only client (no redirect URIs) → client_id. Names
/// must differ between clients in one test: a client_id is its signed
/// registration, so identical registrations are the same client.
async fn register(app: &Router, name: &str) -> String {
    let (status, body) = send(
        app,
        Request::post("/oauth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "client_name": name,
                    "grant_types": [GRANT, "refresh_token"],
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["grant_types"][0], GRANT);
    body["client_id"].as_str().unwrap().to_string()
}

/// Open a grant for `client_id` → the §3.2 response body.
async fn opened(app: &Router, client_id: &str) -> Value {
    let (status, body) = send(
        app,
        Request::post("/oauth/device_authorization")
            .header(header::HOST, "converge.test")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "client_id={}",
                converge_server::oauth::query_encode(client_id)
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// One token-endpoint poll for `device_code`.
async fn poll(app: &Router, client_id: &str, device_code: &str) -> (StatusCode, Value) {
    send(
        app,
        Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "grant_type={}&device_code={}&client_id={}",
                converge_server::oauth::query_encode(GRANT),
                converge_server::oauth::query_encode(device_code),
                converge_server::oauth::query_encode(client_id),
            )))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn pairing_round_trip() {
    let (_pg, _store, app) = server().await;

    // Discovery advertises the flow.
    let (status, meta) = send(
        &app,
        Request::get("/.well-known/oauth-authorization-server")
            .header(header::HOST, "converge.test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        meta["device_authorization_endpoint"],
        "http://converge.test/oauth/device_authorization"
    );
    assert!(
        meta["grant_types_supported"]
            .as_array()
            .unwrap()
            .contains(&json!(GRANT))
    );

    let client_id = register(&app, "converge-cli @ testhost").await;
    let grant = opened(&app, &client_id).await;
    let device_code = grant["device_code"].as_str().unwrap();
    let user_code = grant["user_code"].as_str().unwrap();
    assert_eq!(user_code.len(), 9, "XXXX-XXXX, got {user_code}");
    assert_eq!(
        grant["verification_uri_complete"],
        format!("http://converge.test/#/pair/{user_code}")
    );
    assert_eq!(grant["interval"], 5);

    // Undecided: the poll reports pending.
    let (status, body) = poll(&app, &client_id, device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "authorization_pending");

    // The approval API needs a session; with one, it shows the grant —
    // normalization forgives lowercase, hyphenless entry.
    let (status, _) = send(
        &app,
        Request::get(format!("/api/v1/device/{user_code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let cookie = session(&app).await;
    let sloppy = user_code.replace('-', "").to_lowercase();
    let (status, shown) = send(
        &app,
        Request::get(format!("/api/v1/device/{sloppy}"))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert_eq!(shown["client_name"], "converge-cli @ testhost");

    // Approve. The grant disappears from the approval surface…
    let (status, _) = send(
        &app,
        Request::post(format!("/api/v1/device/{user_code}"))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "approve": true }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(
        &app,
        Request::get(format!("/api/v1/device/{user_code}"))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // …and the poll turns into the token grant: access + refresh, where
    // the refresh token is the CLI's durable credential — an ordinary
    // bearer against the API.
    let (status, granted) = poll(&app, &client_id, device_code).await;
    assert_eq!(status, StatusCode::OK, "{granted}");
    assert_eq!(granted["token_type"], "bearer");
    let refresh = granted["refresh_token"].as_str().unwrap();
    assert!(refresh.starts_with("cvg_"));
    let (status, me) = send(
        &app,
        Request::get("/api/v1/users/me")
            .header(header::AUTHORIZATION, format!("Bearer {refresh}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["handle"], "admin");

    // A device code never yields twice.
    let (status, body) = poll(&app, &client_id, device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "expired_token");
}

#[tokio::test]
async fn denial_and_client_binding() {
    let (_pg, _store, app) = server().await;
    let cookie = session(&app).await;

    // Deny: reported once, then the code is dead.
    let client_id = register(&app, "converge-cli @ testhost").await;
    let grant = opened(&app, &client_id).await;
    let device_code = grant["device_code"].as_str().unwrap();
    let user_code = grant["user_code"].as_str().unwrap();
    let (status, _) = send(
        &app,
        Request::post(format!("/api/v1/device/{user_code}"))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "approve": false }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = poll(&app, &client_id, device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "access_denied");
    let (_, body) = poll(&app, &client_id, device_code).await;
    assert_eq!(body["error"], "expired_token");

    // A grant is bound to the client that opened it: another client
    // polling a stolen device code learns nothing (and doesn't burn it).
    let thief = register(&app, "converge-cli @ elsewhere").await;
    let grant = opened(&app, &client_id).await;
    let device_code = grant["device_code"].as_str().unwrap();
    let (status, body) = poll(&app, &thief, device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "expired_token");
    let (_, body) = poll(&app, &client_id, device_code).await;
    assert_eq!(body["error"], "authorization_pending", "{body}");

    // An unknown user code 404s on the approval surface.
    let (status, _) = send(
        &app,
        Request::get("/api/v1/device/XXXX-XXXX")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
