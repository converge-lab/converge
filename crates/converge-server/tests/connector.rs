//! The MCP-connector OAuth path: discovery, dynamic registration,
//! authorize (browser session), PKCE exchange, the granted tokens against
//! the API and /mcp, refresh, and revocation-as-kill-switch
//! (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{TOKEN, server};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Vec<(String, String)>, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}

fn header_value<'h>(headers: &'h [(String, String)], name: &str) -> &'h str {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("`{name}` header"))
}

/// A session cookie for the harness admin — the "signed-in browser".
async fn session(app: &Router) -> String {
    let (status, headers, _) = send(
        app,
        Request::post("/api/v1/session")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "token": TOKEN }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    header_value(&headers, "set-cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// Registration → client_id.
async fn register(app: &Router) -> String {
    let (status, _, body) = send(
        app,
        Request::post("/oauth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "redirect_uris": [CALLBACK], "client_name": "claude.ai" }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["token_endpoint_auth_method"], "none");
    body["client_id"].as_str().unwrap().to_string()
}

fn authorize_uri(client_id: &str, challenge: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&state=xyz\
         &code_challenge={challenge}&code_challenge_method=S256",
        converge_server::oauth::query_encode(client_id),
        converge_server::oauth::query_encode(CALLBACK),
    )
}

#[tokio::test]
async fn connector_round_trip() {
    let (_pg, store, app) = server().await;

    // Discovery: both well-knowns, endpoints on the request's origin.
    let (status, _, meta) = send(
        &app,
        Request::get("/.well-known/oauth-authorization-server")
            .header(header::HOST, "converge.test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(meta["issuer"], "http://converge.test");
    assert_eq!(meta["token_endpoint"], "http://converge.test/oauth/token");
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));
    let (status, _, meta) = send(
        &app,
        Request::get("/.well-known/oauth-protected-resource/mcp")
            .header(header::HOST, "converge.test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(meta["resource"], "http://converge.test/mcp");

    let client_id = register(&app).await;
    let verifier = "v".repeat(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    // Unauthenticated browser: bounced to sign-in with the resume path.
    let (status, headers, _) = send(
        &app,
        Request::get(authorize_uri(&client_id, &challenge))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let bounce = header_value(&headers, "location");
    assert!(
        bounce.starts_with("/?next=%2Foauth%2Fauthorize"),
        "{bounce}"
    );

    // Signed-in browser: straight back to the connector with a code.
    let cookie = session(&app).await;
    let (status, headers, _) = send(
        &app,
        Request::get(authorize_uri(&client_id, &challenge))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = header_value(&headers, "location").to_string();
    assert!(
        location.starts_with(&format!("{CALLBACK}?code=")),
        "{location}"
    );
    assert!(location.ends_with("&state=xyz"), "{location}");
    let code = location
        .split_once("code=")
        .unwrap()
        .1
        .split('&')
        .next()
        .unwrap()
        .to_string();

    // A wrong verifier buys nothing.
    let form = |verifier: &str| {
        format!(
            "grant_type=authorization_code&code={code}&code_verifier={verifier}\
             &client_id={}&redirect_uri={}",
            converge_server::oauth::query_encode(&client_id),
            converge_server::oauth::query_encode(CALLBACK),
        )
    };
    let (status, _, body) = send(
        &app,
        Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(
                "wrong-verifier-wrong-verifier-wrong-verifier",
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");

    // The right verifier: access + refresh.
    let (status, _, grant) = send(
        &app,
        Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(&verifier)))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{grant}");
    assert_eq!(grant["token_type"], "bearer");
    assert_eq!(grant["expires_in"], 3600);
    let access = grant["access_token"].as_str().unwrap().to_string();
    let refresh = grant["refresh_token"].as_str().unwrap().to_string();
    assert!(refresh.starts_with("cvg_"));

    // The access token is a working credential on the API and /mcp.
    let (status, _, me) = send(
        &app,
        Request::get("/api/v1/users/me")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["handle"], "admin");
    let (status, _, tools) = send(
        &app,
        Request::post("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::HOST, "127.0.0.1")
            .body(Body::from(
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })
                    .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);

    // The refresh grant mints a fresh access token…
    let (status, _, refreshed) = send(
        &app,
        Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "grant_type=refresh_token&refresh_token={refresh}"
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert!(refreshed["access_token"].as_str().is_some());

    // …and the connector shows up as a revocable token; revoking it ends
    // the refresh line while the short-lived access token merely expires.
    use converge_storage::{Pagination, Tokens, Users};
    let admin = store
        .user_login(converge_storage::Identity {
            provider: "local".into(),
            subject: "admin".into(),
            handle: "admin".into(),
            name: "Admin".into(),
        })
        .await
        .unwrap();
    let tokens = store
        .token_list(admin, Pagination::default())
        .await
        .unwrap();
    let connector = tokens
        .iter()
        .find(|t| t.label == "connector:claude.ai")
        .expect("the connector's refresh token is listed");
    store.token_revoke(admin, connector.id).await.unwrap();
    let (status, _, body) = send(
        &app,
        Request::post("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "grant_type=refresh_token&refresh_token={refresh}"
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
}

#[tokio::test]
async fn authorize_rejects_what_the_code_would_leak() {
    let (_pg, _store, app) = server().await;
    let cookie = session(&app).await;
    let client_id = register(&app).await;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(b"v"));

    // A redirect target the client never registered.
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}\
         &code_challenge={challenge}&code_challenge_method=S256",
        converge_server::oauth::query_encode(&client_id),
        converge_server::oauth::query_encode("https://evil.example/cb"),
    );
    let (status, _, _) = send(
        &app,
        Request::get(uri)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // PKCE is not optional.
    let uri = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}",
        converge_server::oauth::query_encode(&client_id),
        converge_server::oauth::query_encode(CALLBACK),
    );
    let (status, _, _) = send(
        &app,
        Request::get(uri)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
