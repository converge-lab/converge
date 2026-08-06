//! Shared test harness: a fresh Postgres per test plus the app router
//! (testcontainers — needs Docker).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use converge_server::auth::Sessions;
use converge_server::{Expert, app, auth};
use converge_storage::{AgentKind, Agents, Identity, NewAgent, Tokens, Users};
use converge_storage_postgres::PgStorage;
use http_body_util::BodyExt;
use serde_json::Value;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tower::ServiceExt;

/// The bearer secret every harness request presents (hashed into the
/// store at boot, exactly like the real bootstrap flow).
pub const TOKEN: &str = "cvg_test";

/// Boot a fresh Postgres, migrate, log the admin in with a token, build
/// the app. The container lives as long as the returned handle. The store
/// handle lets tests seed around surfaces the API deliberately doesn't
/// expose yet (users/agents).
pub async fn server() -> (ContainerAsync<Postgres>, PgStorage, Router) {
    hosted(None).await
}

/// [`server`] with a configured public origin (`auth.public_url`) — for
/// suites exercising surfaces that depend on the deployment's external
/// name (the /mcp Host guard, issued URLs).
#[allow(dead_code)] // pulled in per-suite; not every suite needs an origin
pub async fn hosted(public: Option<&str>) -> (ContainerAsync<Postgres>, PgStorage, Router) {
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
    let admin = store.user_login(me.clone()).await.unwrap();
    store
        .token_add(admin, "test".into(), auth::hash(TOKEN))
        .await
        .unwrap();
    // No expert jobs configured: detection is a no-op in the harness (the
    // dedicated suite builds its own registry against a stub endpoint).
    let agent = store
        .agent_ensure(NewAgent {
            kind: AgentKind::Model,
            name: "expert".into(),
        })
        .await
        .unwrap();
    let expert = Expert::new(store.clone(), converge_expert::Registry::default(), agent);
    (
        node,
        store.clone(),
        app(
            store,
            Sessions::new(Some("test-session-secret")),
            None,
            public.map(str::to_string),
            None,
            expert,
        ),
    )
}

/// Send one request as the admin; return status and parsed JSON body
/// (`null` when empty).
// Not every suite that pulls in the harness uses the authed helper (the
// session suite sends raw requests on purpose).
#[allow(dead_code)]
pub async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_as(app, TOKEN, method, uri, body).await
}

/// [`send`], but presenting a specific bearer token — the ACL suite
/// speaks as several users.
#[allow(dead_code)]
pub async fn send_as(
    app: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        // rmcp's streamable-HTTP transport insists on the Accept pair and
        // a Host header (DNS-rebinding protection); harmless for REST.
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::HOST, "127.0.0.1")
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let request = match body {
        Some(v) => request.body(Body::from(v.to_string())).unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "non-JSON response ({e}): {:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, value)
}
