//! The IdP sign-in flow against a mock OIDC provider: discovery, redirect,
//! state/PKCE round trip, code exchange, userinfo, allowlist, session
//! (testcontainers — needs Docker).

use axum::body::Body;
use axum::extract::Form;
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_server::auth::Sessions;
use converge_server::oidc::{Oidc, Settings};
use converge_storage::Identity;
use converge_storage_postgres::PgStorage;
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

/// A minimal OIDC IdP: discovery + token + userinfo, asserting the flow
/// details a real provider would (grant type, PKCE verifier, the bearer).
async fn idp() -> String {
    #[derive(Deserialize)]
    struct Exchange {
        grant_type: String,
        code: String,
        code_verifier: String,
    }
    async fn token(Form(form): Form<Exchange>) -> Json<serde_json::Value> {
        assert_eq!(form.grant_type, "authorization_code");
        assert_eq!(form.code, "good-code");
        assert!(!form.code_verifier.is_empty(), "PKCE verifier must travel");
        Json(json!({ "access_token": "at-123", "token_type": "Bearer" }))
    }
    async fn userinfo(request: Request<Body>) -> Json<serde_json::Value> {
        let bearer = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert_eq!(bearer, "Bearer at-123");
        Json(json!({
            "sub": "u-42",
            "preferred_username": "alice",
            "name": "Alice A.",
            "email": "alice@example.com",
        }))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let well_known = {
        let issuer = issuer.clone();
        get(move || async move {
            Json(json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"),
                "userinfo_endpoint": format!("{issuer}/userinfo"),
            }))
        })
    };
    let router = Router::new()
        .route("/.well-known/openid-configuration", well_known)
        .route("/token", post(token))
        .route("/userinfo", get(userinfo));
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    issuer
}

/// The converge app wired to the mock IdP.
async fn server(
    issuer: &str,
    allowed: Option<Vec<String>>,
) -> (
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    Router,
) {
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::ImageExt;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    let node = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start postgres (is Docker running?)");
    let port = node.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStorage::connect(&url).await.unwrap();
    store.migrate().await.unwrap();

    let me = Identity {
        provider: "local".into(),
        subject: "admin".into(),
        handle: "admin".into(),
        name: "Admin".into(),
    };
    let oidc = Oidc::new(Settings {
        provider: "corp".into(),
        issuer: Some(issuer.into()),
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        public_url: "http://127.0.0.1:8080".into(),
        allowed,
    });
    let app = converge_server::app(
        store,
        me,
        Sessions::new(Some("test-session-secret")),
        Some(oidc),
        None,
        None,
    );
    (node, app)
}

/// Split one `k=v` pair out of a query string.
fn query_param(url: &str, key: &str) -> String {
    let (_, query) = url.split_once('?').expect("a query string");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("`{key}` in {url}"))
        .to_string()
}

#[tokio::test]
async fn sign_in_round_trip() {
    let issuer = idp().await;
    let (_pg, app) = server(&issuer, None).await;

    // The capability read says the button may be shown.
    let response = app
        .clone()
        .oneshot(Request::get("/api/v1/auth").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let info = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&info).unwrap()["oidc"],
        "corp"
    );

    // /auth/login: redirect to the provider, flow cookie set.
    let response = app
        .clone()
        .oneshot(Request::get("/auth/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(location.starts_with(&format!("{issuer}/authorize?")));
    let state = query_param(location, "state");
    let flow = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(flow.starts_with("converge_oauth="), "{flow}");
    assert!(flow.contains("HttpOnly"), "{flow}");
    assert!(flow.contains("SameSite=Lax"), "{flow}");
    let flow_pair = flow.split(';').next().unwrap().to_string();

    // The provider redirects back: code exchange + userinfo + session.
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/callback?code=good-code&state={state}"))
                .header(header::COOKIE, &flow_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let session = cookies
        .iter()
        .find(|c| c.starts_with("converge_session="))
        .expect("a session cookie");
    assert!(session.contains("HttpOnly"), "{session}");
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("converge_oauth=") && c.contains("Max-Age=0")),
        "the flow cookie must be retired: {cookies:?}"
    );

    // The session belongs to the IdP-asserted identity.
    let session_pair = session.split(';').next().unwrap().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/users/me")
                .header(header::COOKIE, &session_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let me = response.into_body().collect().await.unwrap().to_bytes();
    let me: serde_json::Value = serde_json::from_slice(&me).unwrap();
    assert_eq!(me["provider"], "corp");
    assert_eq!(me["subject"], "u-42");
    assert_eq!(me["handle"], "alice");
    assert_eq!(me["name"], "Alice A.");

    // A tampered state is rejected before any exchange.
    let response = app
        .clone()
        .oneshot(
            Request::get("/auth/callback?code=good-code&state=forged")
                .header(header::COOKIE, &flow_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn allowlist_turns_the_identity_away() {
    let issuer = idp().await;
    let (_pg, app) = server(&issuer, Some(vec!["someone-else".into()])).await;

    let response = app
        .clone()
        .oneshot(Request::get("/auth/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    let state = query_param(location, "state");
    let flow = response.headers()[header::SET_COOKIE].to_str().unwrap();
    let flow_pair = flow.split(';').next().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/auth/callback?code=good-code&state={state}"))
                .header(header::COOKIE, &flow_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The IdP said yes; this deployment says no — and no session exists.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().get(header::SET_COOKIE).is_none_or(|c| {
        !c.to_str()
            .unwrap_or_default()
            .starts_with("converge_session=")
    }));
}
