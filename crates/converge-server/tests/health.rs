//! The app boots over a real Postgres and answers the health probe
//! (testcontainers — needs Docker).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use converge_server::app;
use converge_server::auth::Sessions;
use converge_storage::Identity;
use converge_storage_postgres::PgStorage;
use http_body_util::BodyExt;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tower::ServiceExt;

#[tokio::test]
async fn healthz() {
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
    let response = app(
        store.clone(),
        me.clone(),
        Sessions::new(Some("test-session-secret")),
        None,
        None,
    )
    .oneshot(Request::get("/api/v1/healthz").body(Body::empty()).unwrap())
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Auth is always on: no token and bad tokens are 401 everywhere but
    // healthz; unknown paths under the gate answer 401 before 404.
    let gate = app(
        PgStorage::connect(&url).await.unwrap(),
        me.clone(),
        Sessions::new(Some("test-session-secret")),
        None,
        None,
    );
    for (uri, token) in [
        ("/api/v1/groups", None),
        ("/api/v1/groups", Some("Bearer cvg_wrong")),
        ("/api/v1/nope", None),
    ] {
        let mut request = Request::get(uri);
        if let Some(t) = token {
            request = request.header("authorization", t);
        }
        let response = gate
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} {token:?}"
        );
    }

    // With a dist directory configured, assets serve same-origin and the
    // hash-routed app falls back to index.html; the API keeps priority.
    let dist = std::env::temp_dir().join(format!("converge-dist-{}", std::process::id()));
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<title>Converge</title>").unwrap();
    let web = app(
        store,
        me,
        Sessions::new(Some("test-session-secret")),
        None,
        Some(&dist),
    );
    for uri in ["/", "/anything-else"] {
        let response = web
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            String::from_utf8_lossy(&bytes).contains("Converge"),
            "{uri}"
        );
    }
    let api = web
        .clone()
        .oneshot(Request::get("/api/v1/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::OK);
    std::fs::remove_dir_all(&dist).ok();
}
