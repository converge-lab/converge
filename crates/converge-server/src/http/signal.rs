//! `/api/v1/signals` — typed decision → decisions observations, over the
//! [`Signals`] trait.
//!
//! Signals are born `proposed`; `PATCH /signals/{id}` resolves one to
//! `confirmed` or `dismissed`, stamping who judged it. A decision's
//! signals (either end) live at `/decisions/{id}/signals` — a read-only
//! relation projection, per the REST-shape decision.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    Author, DecisionId, NewSignal, Page, Pagination, Signal, SignalFilter, SignalId, SignalStatus,
    Storage, StoreError,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/signals", post(add::<S>).get(list::<S>))
        .route("/api/v1/signals/{id}", get(fetch::<S>).patch(resolve::<S>))
        .route("/api/v1/decisions/{id}/signals", get(by_decision::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewSignal>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.signal_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by `?project=&decision=&status=&tier=` (project and
/// decision match either end), paged by `?limit=&cursor=`.
async fn list<S: Storage>(
    State(store): State<S>,
    Query(filter): Query<SignalFilter>,
    Query(page): Query<Pagination<SignalId>>,
) -> Result<Json<Page<Signal>>> {
    let items = store.signal_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |s| s.id.to_string())))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Path(id): Path<SignalId>,
) -> Result<Json<Signal>> {
    Ok(Json(
        store.signal_get(id).await?.ok_or(StoreError::NotFound)?,
    ))
}

/// The resolution: a verdict and who judged it.
#[derive(Deserialize)]
struct Resolve {
    status: SignalStatus,
    by: Author,
}

async fn resolve<S: Storage>(
    State(store): State<S>,
    Path(id): Path<SignalId>,
    Json(resolve): Json<Resolve>,
) -> Result<StatusCode> {
    store.signal_resolve(id, resolve.status, resolve.by).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Read-only relation projection: signals touching one decision on either
/// end (the canonical form stays `/signals?decision=`). The bound parent
/// must exist — an unknown decision is 404, not `[]`.
async fn by_decision<S: Storage>(
    State(store): State<S>,
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
    store.decision_get(id).await?.ok_or(StoreError::NotFound)?;
    filter.decision = Some(id);
    let items = store.signal_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |s| s.id.to_string())))
}
