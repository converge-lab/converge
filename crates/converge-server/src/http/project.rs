//! `/api/v1/projects` — CRUD over the [`Projects`] trait.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    NewProject, Project, ProjectEdit, ProjectFilter, ProjectId, Storage, StoreError,
};
use serde_json::{Value, json};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/projects", post(add::<S>).get(list::<S>))
        .route("/api/v1/projects/{id}", get(fetch::<S>).patch(edit::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewProject>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.project_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by the filter (`?group=<id>&limit=<n>`).
async fn list<S: Storage>(
    State(store): State<S>,
    Query(filter): Query<ProjectFilter>,
) -> Result<Json<Vec<Project>>> {
    Ok(Json(store.project_list(filter).await?))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Path(id): Path<ProjectId>,
) -> Result<Json<Project>> {
    Ok(Json(
        store.project_get(id).await?.ok_or(StoreError::NotFound)?,
    ))
}

async fn edit<S: Storage>(
    State(store): State<S>,
    Path(id): Path<ProjectId>,
    Json(edits): Json<Vec<ProjectEdit>>,
) -> Result<StatusCode> {
    store.project_edit(id, edits).await?;
    Ok(StatusCode::NO_CONTENT)
}
