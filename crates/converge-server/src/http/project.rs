//! `/api/v1/projects` — CRUD over the [`Projects`] trait.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    GroupId, NewProject, Pagination, Project, ProjectEdit, ProjectFilter, ProjectId, Storage,
    StoreError,
};
use serde_json::{Value, json};

use super::error::Result;
use super::page::Page;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/projects", post(add::<S>).get(list::<S>))
        .route("/api/v1/projects/{id}", get(fetch::<S>).patch(edit::<S>))
        .route("/api/v1/groups/{id}/projects", get(by_group::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewProject>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.project_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by the filter (`?group=`), paged by `?limit=&cursor=`.
async fn list<S: Storage>(
    State(store): State<S>,
    Query(filter): Query<ProjectFilter>,
    Query(page): Query<Pagination<ProjectId>>,
) -> Result<Json<Page<Project>>> {
    let items = store.project_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |p| p.id.to_string())))
}

/// Read-only relation projection: the flat list with the group bound by
/// the path (the canonical form stays `/projects?group=`). Unlike the flat
/// filter, the bound parent must exist — an unknown group is 404, not `[]`.
async fn by_group<S: Storage>(
    State(store): State<S>,
    Path(id): Path<GroupId>,
    Query(mut filter): Query<ProjectFilter>,
    Query(page): Query<Pagination<ProjectId>>,
) -> Result<Json<Page<Project>>> {
    if filter.group.is_some() {
        return Err(StoreError::Invalid(
            "group is bound by the path; drop the query parameter".into(),
        )
        .into());
    }
    store.group_get(id).await?.ok_or(StoreError::NotFound)?;
    filter.group = Some(id);
    let items = store.project_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |p| p.id.to_string())))
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
