//! Shared test harness: a fresh Postgres per test plus the app router
//! (testcontainers — needs Docker).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use converge_server::app;
use converge_storage::NewUser;
use converge_storage_postgres::PgStorage;
use http_body_util::BodyExt;
use serde_json::Value;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tower::ServiceExt;

/// Boot a fresh Postgres, migrate, build the app. The container lives as
/// long as the returned handle. The store handle lets tests seed around
/// surfaces the API deliberately doesn't expose yet (users/agents).
pub async fn server() -> (ContainerAsync<Postgres>, PgStorage, Router) {
    let node = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start postgres (is Docker running?)");
    let port = node.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStorage::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let me = NewUser {
        handle: "admin".into(),
        name: "Admin".into(),
    };
    (node, store.clone(), app(store, me))
}

/// Send one request; return status and parsed JSON body (`null` when empty).
pub async fn send(
    app: &Router,
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
        .header(header::HOST, "127.0.0.1");
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
