//! `/api/v1/signals` — typed decision → decisions observations, over the
//! [`Signals`] trait.
//!
//! Signals are born `proposed`; `PATCH /signals/{id}` resolves one to
//! `confirmed` or `dismissed`, stamping who judged it. A decision's
//! signals (either end) live at `/decisions/{id}/signals` — a read-only
//! relation projection, per the REST-shape decision.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    Author, DecisionId, NewSignal, Page, Pagination, ProjectId, Scope, Signal, SignalFilter,
    SignalId, SignalStatus, Storage, StoreError, Tier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/signals", post(add::<S>).get(list::<S>))
        // A claim is not a sub-collection of one signal, so it does not
        // hang off `/signals`: it is its own thing, beside the receipts
        // that are its other half.
        .route("/api/v1/claims", post(claim::<S>))
        .route("/api/v1/signals/{id}", get(fetch::<S>).patch(resolve::<S>))
        .route("/api/v1/decisions/{id}/signals", get(by_decision::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewSignal>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.signal_add(Scope::User(caller.user), new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by `?project=&decision=&status=&tier=` (project and
/// decision match either end), paged by `?limit=&cursor=`.
async fn list<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Query(filter): Query<SignalFilter>,
    Query(page): Query<Pagination<SignalId>>,
) -> Result<Json<Page<Signal>>> {
    let items = store
        .signal_list(Scope::User(caller.user), filter, page.clone())
        .await?;
    Ok(Json(Page::new(items, &page, |s| s.id.to_string())))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<SignalId>,
) -> Result<Json<Signal>> {
    Ok(Json(
        store
            .signal_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}

/// A session asking what it has not been shown.
#[derive(Deserialize)]
struct Claim {
    /// The harness's session id: the receipts key, with the caller.
    session: String,
    /// claude | codex | opencode | cursor.
    #[serde(default)]
    harness: Option<String>,
    /// The project this session is working in. Only signals touching it
    /// on either end are handed over. Omitted — as clients released
    /// before this do — every visible group is claimed from.
    #[serde(default)]
    project: Option<ProjectId>,
    /// At most this many, oldest first; the ledger advances past what is
    /// returned, and only that. Capped, so a client cannot drain a burst
    /// into one prompt.
    #[serde(default)]
    limit: Option<u32>,
    /// The lowest tier worth showing now. Lower ones are consumed all
    /// the same: they stay in the session-start listing and in
    /// `signal_list`, and are not offered to this session again.
    #[serde(default)]
    floor: Option<Tier>,
}

/// What a claim handed over, and the frame that puts it in front of the
/// model — absent when nothing is worth showing.
#[derive(Serialize)]
struct Claimed {
    signals: Vec<Signal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<String>,
}

const CLAIM_DEFAULT: u32 = 3;
const CLAIM_CAP: u32 = 20;

/// What one (caller, session) has not been shown: proposed signals
/// recorded after the session began, with no receipt for it, oldest
/// first, receipted as they go. A claim is the resource — the read has
/// a side effect, the receipts, so it cannot be a `GET`, and the
/// answer is what this claim contains. A session first seen here opens its row
/// and gets `[]`. This is the poll's
/// endpoint — a hook on the harness's per-prompt seam calls it — and it
/// is where deliveries and their age are counted.
async fn claim<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(claim): Json<Claim>,
) -> Result<Json<Claimed>> {
    let limit = claim.limit.unwrap_or(CLAIM_DEFAULT).min(CLAIM_CAP);
    let signals = store
        .signal_claim(
            Scope::User(caller.user),
            &claim.session,
            claim.harness.as_deref(),
            claim.project,
            limit,
        )
        .await?;
    let floor = claim.floor.unwrap_or(Tier::Watch);
    let signals: Vec<Signal> = signals.into_iter().filter(|s| s.tier >= floor).collect();
    crate::metrics::delivered("poll", &signals);
    let (context, line) = crate::context::signals(&signals).unzip();
    Ok(Json(Claimed {
        signals,
        context,
        line,
    }))
}

/// The resolution: a verdict and who judged it.
#[derive(Deserialize)]
struct Resolve {
    status: SignalStatus,
    by: Author,
}

async fn resolve<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<SignalId>,
    Json(resolve): Json<Resolve>,
) -> Result<StatusCode> {
    store
        .signal_resolve(Scope::User(caller.user), id, resolve.status, resolve.by)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Read-only relation projection: signals touching one decision on either
/// end (the canonical form stays `/signals?decision=`). The bound parent
/// must exist — an unknown decision is 404, not `[]`.
async fn by_decision<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
    Query(mut filter): Query<SignalFilter>,
    Query(page): Query<Pagination<SignalId>>,
) -> Result<Json<Page<Signal>>> {
    if filter.decision.is_some() || filter.project.is_some() {
        return Err(StoreError::Invalid(
            "the decision is bound by the path; drop the decision/project query parameters".into(),
        )
        .into());
    }
    let scope = Scope::User(caller.user);
    store
        .decision_get(scope, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    filter.decision = Some(id);
    let items = store.signal_list(scope, filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |s| s.id.to_string())))
}
