//! `POST /api/v1/receipts` — what a session was shown, both kinds in one
//! call. A session start lists decisions and signals together and has a
//! one-second budget for saying so, which is why this is one round trip
//! rather than two. `session = ""` is the person reading on the web.
//!
//! `/api/v1/signals/receipts` (signals only) stays beside it: CLIs
//! released before this endpoint existed call it, and a receipt is not
//! worth breaking an older install over.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};
use converge_storage::{DecisionId, Scope, SignalId, Storage};
use serde::Deserialize;

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new().route("/api/v1/receipts", post(receive::<S>))
}

#[derive(Deserialize)]
struct Receipts {
    /// The harness's session id, or `""` for the person on the web.
    session: String,
    /// claude | codex | opencode | cursor, for a harness session.
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    signal_ids: Vec<SignalId>,
    #[serde(default)]
    decision_ids: Vec<DecisionId>,
}

/// Record both kinds. Ids the caller cannot see are ignored, so a
/// receipt never becomes a claim about something they were not shown.
async fn receive<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(receipts): Json<Receipts>,
) -> Result<StatusCode> {
    let scope = Scope::User(caller.user);
    let harness = receipts.harness.as_deref();
    // Signals first: the session row is created by whichever runs first,
    // and both calls are idempotent.
    store
        .signal_receive(scope, &receipts.session, harness, &receipts.signal_ids)
        .await?;
    store
        .decision_receive(scope, &receipts.session, harness, &receipts.decision_ids)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
