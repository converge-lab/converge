//! The HTTP surface: the versioned web API under `/api/v1`.
//!
//! One module per resource, mirroring the storage crate; the `error` module
//! carries the `StoreError` → status mapping they all share; the pagination
//! envelope (`Page`) comes from the storage crate — it's part of the wire
//! contract shared with `converge-client`.

mod agent;
mod decision;
mod error;
mod evidence;
mod group;
mod oauth;
mod project;
mod session;
mod signin;
mod token;
mod user;

use std::path::Path;
use std::sync::Arc;

use axum::routing::get;
use axum::{Router, middleware};
use converge_storage::{Identity, Storage};
use tower_http::services::{ServeDir, ServeFile};

use crate::auth::Sessions;
use crate::oidc::Oidc;

/// The application router over any storage backend: the versioned web API
/// plus the unversioned, stateless `/mcp` endpoint — both behind
/// authentication (`crate::auth`: bearer token or session cookie), no
/// fallback caller. `me` is the deployment identity MCP authorship stamps
/// in single-user mode. Open paths: `healthz`, the session exchange
/// (`/api/v1/session` — the gate's entrance), and, when `web` names a
/// trunk `dist/` directory, the static assets served same-origin as the
/// fallback (the app must load to show its login screen; it is
/// hash-routed, so `/` → `index.html` suffices).
pub fn app<S: Storage + 'static>(
    store: S,
    me: Identity,
    sessions: Sessions,
    oidc: Option<Oidc>,
    public: Option<String>,
    web: Option<&Path>,
) -> Router {
    let issuer = oauth::Issuer {
        store: store.clone(),
        sessions: sessions.clone(),
        oauth: crate::oauth::Oauth::new(sessions.clone()),
        public,
        signin: oidc.is_some(),
    };
    let oidc = Arc::new(oidc);
    let protected = Router::new()
        .merge(group::routes().with_state(store.clone()))
        .merge(project::routes().with_state(store.clone()))
        .merge(decision::routes().with_state(store.clone()))
        .merge(evidence::routes().with_state(store.clone()))
        .merge(agent::routes().with_state(store.clone()))
        .merge(token::routes().with_state(store.clone()))
        .merge(user::routes().with_state(store.clone()))
        .nest_service("/mcp", crate::mcp::service(store.clone(), me))
        .layer(middleware::from_fn_with_state(
            (store.clone(), sessions.clone()),
            crate::auth::require::<S>,
        ));
    let router = Router::new()
        .route("/api/v1/healthz", get(healthz))
        .merge(oauth::routes().with_state(issuer))
        .merge(signin::routes().with_state((store.clone(), sessions.clone(), oidc)))
        .merge(session::routes().with_state((store, sessions)))
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
async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        // The workspace version — clients compare it against their own
        // build to surface skew (the CLI is a distributed binary).
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
