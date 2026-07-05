//! The app boots over a real Postgres and answers the health probe
//! (testcontainers — needs Docker).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use converge_server::app;
use converge_storage::NewUser;
use converge_storage_postgres::PgStorage;
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

    let me = NewUser {
        handle: "admin".into(),
        name: "Admin".into(),
    };
    let response = app(store, me.clone())
        .oneshot(Request::get("/api/v1/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let missing = app(PgStorage::connect(&url).await.unwrap(), me)
        .oneshot(Request::get("/api/v1/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
