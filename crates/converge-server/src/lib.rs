//! The Converge server — the product's HTTP surface over the storage seam.
//!
//! The versioned web API lives under `/api/v1`; the MCP endpoint (`/mcp`,
//! unversioned, stateless) lands in a later slice. Everything is written
//! against the `converge_storage` traits, never a concrete backend — the
//! binary picks the backend (PostgreSQL) at the edge.

use axum::Router;
use axum::routing::get;
use converge_storage::Storage;

/// The application router over any storage backend.
pub fn app<S: Storage + 'static>(store: S) -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .with_state(store)
}

/// Process liveness only. Storage connectivity is proven at startup
/// (connect + migrate); a readiness probe can come when something needs it.
async fn healthz() -> &'static str {
    "ok"
}
