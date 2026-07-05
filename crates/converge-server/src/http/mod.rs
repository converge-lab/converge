//! The HTTP surface: the versioned web API under `/api/v1`.
//!
//! One module per resource, mirroring the storage crate; the `error` module
//! carries the `StoreError` → status mapping and `page` the pagination
//! envelope they all share.

mod decision;
mod error;
mod group;
mod page;
mod project;
mod user;

use axum::Router;
use axum::routing::get;
use converge_storage::{NewUser, Storage};

/// The application router over any storage backend. `me` is the identity
/// `/api/v1/users/me` resolves to in single-user mode.
pub fn app<S: Storage + 'static>(store: S, me: NewUser) -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .merge(group::routes().with_state(store.clone()))
        .merge(project::routes().with_state(store.clone()))
        .merge(decision::routes().with_state(store.clone()))
        .merge(user::routes().with_state((store, me)))
}

/// Process liveness only. Storage connectivity is proven at startup
/// (connect + migrate); a readiness probe can come when something needs it.
async fn healthz() -> &'static str {
    "ok"
}
