//! The HTTP surface: the versioned web API under `/api/v1`.
//!
//! One module per resource, mirroring the storage crate; the `error` module
//! carries the `StoreError` → status mapping they all share; the pagination
//! envelope (`Page`) comes from the storage crate — it's part of the wire
//! contract shared with `converge-client`.

mod agent;
mod decision;
mod error;
mod group;
mod project;
mod user;

use std::path::Path;

use axum::routing::get;
use axum::{Router, middleware};
use converge_storage::{Identity, Storage};
use tower_http::services::{ServeDir, ServeFile};

/// The application router over any storage backend: the versioned web API
/// plus the unversioned, stateless `/mcp` endpoint — both behind bearer
/// authentication (`crate::auth`), no fallback caller. `me` is the
/// deployment identity MCP authorship stamps in single-user mode. Open
/// paths: `healthz` and, when `web` names a trunk `dist/` directory, the
/// static assets served same-origin as the fallback (the app must load to
/// show a login screen; it is hash-routed, so `/` → `index.html`
/// suffices).
pub fn app<S: Storage + 'static>(store: S, me: Identity, web: Option<&Path>) -> Router {
    let protected = Router::new()
        .merge(group::routes().with_state(store.clone()))
        .merge(project::routes().with_state(store.clone()))
        .merge(decision::routes().with_state(store.clone()))
        .merge(agent::routes().with_state(store.clone()))
        .merge(user::routes().with_state(store.clone()))
        .nest_service("/mcp", crate::mcp::service(store.clone(), me))
        .layer(middleware::from_fn_with_state(
            store,
            crate::auth::require::<S>,
        ));
    let router = Router::new()
        .route("/api/v1/healthz", get(healthz))
        .merge(protected);
    match web {
        Some(dist) => router.fallback_service(
            ServeDir::new(dist).fallback(ServeFile::new(dist.join("index.html"))),
        ),
        None => router,
    }
}

/// Process liveness only. Storage connectivity is proven at startup
/// (connect + migrate); a readiness probe can come when something needs it.
async fn healthz() -> &'static str {
    "ok"
}
