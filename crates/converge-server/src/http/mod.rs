//! The HTTP surface: the versioned web API under `/api/v1`.
//!
//! One module per resource, mirroring the storage crate; the `error` module
//! carries the `StoreError` → status mapping they all share.

mod error;
mod group;
mod project;

use axum::Router;
use axum::routing::get;
use converge_storage::Storage;

/// The application router over any storage backend.
pub fn app<S: Storage + 'static>(store: S) -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .merge(group::routes())
        .merge(project::routes())
        .with_state(store)
}

/// Process liveness only. Storage connectivity is proven at startup
/// (connect + migrate); a readiness probe can come when something needs it.
async fn healthz() -> &'static str {
    "ok"
}
