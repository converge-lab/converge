//! `/api/v1/groups` — CRUD over the [`Groups`] trait.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{Group, GroupEdit, GroupId, NewGroup, Pagination, Storage, StoreError};
use serde_json::{Value, json};

use super::error::Result;
use super::page::Page;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/groups", post(add::<S>).get(list::<S>))
        .route("/api/v1/groups/{id}", get(fetch::<S>).patch(edit::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewGroup>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.group_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Query(page): Query<Pagination<GroupId>>,
) -> Result<Json<Page<Group>>> {
    let items = store.group_list(page.clone()).await?;
    Ok(Json(Page::new(items, &page, |g| g.id.to_string())))
}

async fn fetch<S: Storage>(State(store): State<S>, Path(id): Path<GroupId>) -> Result<Json<Group>> {
    Ok(Json(
        store.group_get(id).await?.ok_or(StoreError::NotFound)?,
    ))
}

async fn edit<S: Storage>(
    State(store): State<S>,
    Path(id): Path<GroupId>,
    Json(edits): Json<Vec<GroupEdit>>,
) -> Result<StatusCode> {
    store.group_edit(id, edits).await?;
    Ok(StatusCode::NO_CONTENT)
}
