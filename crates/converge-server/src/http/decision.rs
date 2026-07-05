//! `/api/v1/decisions` — CRUD, the atomic edit batch, and the graph edges,
//! over the [`Decisions`] trait.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, Edges, NewDecision, ProjectId, Storage,
    StoreError,
};
use serde_json::{Value, json};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/decisions", post(add::<S>).get(list::<S>))
        .route("/api/v1/decisions/{id}", get(fetch::<S>).patch(edit::<S>))
        .route("/api/v1/decisions/{id}/edges", get(edges::<S>))
        .route("/api/v1/projects/{id}/decisions", get(by_project::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewDecision>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.decision_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by the filter
/// (`?project=<id>&group=<id>&status=<status>&limit=<n>`). Status matches
/// the *derived* status — `superseded` finds decisions with inbound edges.
async fn list<S: Storage>(
    State(store): State<S>,
    Query(filter): Query<DecisionFilter>,
) -> Result<Json<Vec<Decision>>> {
    Ok(Json(store.decision_list(filter).await?))
}

/// Read-only relation projection: the flat list with the project bound by
/// the path (the canonical form stays `/decisions?project=`). Unlike the
/// flat filter, the bound parent must exist — an unknown project is 404,
/// not `[]`.
async fn by_project<S: Storage>(
    State(store): State<S>,
    Path(id): Path<ProjectId>,
    Query(mut filter): Query<DecisionFilter>,
) -> Result<Json<Vec<Decision>>> {
    if filter.project.is_some() || filter.group.is_some() {
        return Err(StoreError::Invalid(
            "project is bound by the path; drop the project/group query parameters".into(),
        )
        .into());
    }
    store.project_get(id).await?.ok_or(StoreError::NotFound)?;
    filter.project = Some(id);
    Ok(Json(store.decision_list(filter).await?))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Path(id): Path<DecisionId>,
) -> Result<Json<Decision>> {
    Ok(Json(
        store.decision_get(id).await?.ok_or(StoreError::NotFound)?,
    ))
}

async fn edit<S: Storage>(
    State(store): State<S>,
    Path(id): Path<DecisionId>,
    Json(edits): Json<Vec<DecisionEdit>>,
) -> Result<StatusCode> {
    store.decision_edit(id, edits).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The direct graph neighbourhood of one decision, both directions.
async fn edges<S: Storage>(
    State(store): State<S>,
    Path(id): Path<DecisionId>,
) -> Result<Json<Edges>> {
    Ok(Json(
        store
            .decision_edges(id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}
