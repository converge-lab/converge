//! `/api/v1/projects` — CRUD over the [`Projects`] trait, caller-scoped
//! through the owning group.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    GroupId, NewProject, Page, Pagination, Project, ProjectEdit, ProjectFilter, ProjectId,
    Repository, Scope, Storage, StoreError,
};
use serde_json::{Value, json};

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/projects", post(add::<S>).get(list::<S>))
        .route(
            "/api/v1/projects/{id}",
            get(fetch::<S>).patch(edit::<S>).delete(remove::<S>),
        )
        .route("/api/v1/groups/{id}/projects", get(by_group::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewProject>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.project_add(Scope::User(caller.user), new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// List, narrowed by the filter (`?group=`), paged by `?limit=&cursor=`.
async fn list<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Query(filter): Query<ProjectFilter>,
    Query(page): Query<Pagination<ProjectId>>,
) -> Result<Json<Page<Project>>> {
    let items = store
        .project_list(Scope::User(caller.user), filter, page.clone())
        .await?;
    Ok(Json(Page::new(items, &page, |p| p.id.to_string())))
}

/// Read-only relation projection: the flat list with the group bound by
/// the path (the canonical form stays `/projects?group=`). Unlike the flat
/// filter, the bound parent must exist — an unknown group is 404, not `[]`.
async fn by_group<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
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
    let scope = Scope::User(caller.user);
    store
        .group_get(scope, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    filter.group = Some(id);
    let items = store.project_list(scope, filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |p| p.id.to_string())))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<ProjectId>,
) -> Result<Json<Project>> {
    Ok(Json(
        store
            .project_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}

/// A project's own fields. Absent leaves one alone; `null` clears a
/// description or unsets the repository.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "super::nullable")]
    description: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::nullable")]
    repository: Option<Option<Repository>>,
    /// Whether whole conversations are kept here. Owner-only, which
    /// storage enforces.
    #[serde(default)]
    archive_transcripts: Option<bool>,
}

impl Patch {
    fn edits(self) -> Vec<ProjectEdit> {
        let mut edits = Vec::new();
        if let Some(name) = self.name {
            edits.push(ProjectEdit::SetName(name));
        }
        if let Some(description) = self.description {
            edits.push(ProjectEdit::SetDescription(description));
        }
        if let Some(repository) = self.repository {
            edits.push(ProjectEdit::SetRepository(repository));
        }
        if let Some(keep) = self.archive_transcripts {
            edits.push(ProjectEdit::SetArchiveTranscripts(keep));
        }
        edits
    }
}

async fn edit<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<ProjectId>,
    Json(patch): Json<Patch>,
) -> Result<StatusCode> {
    store
        .project_edit(Scope::User(caller.user), id, patch.edits())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<ProjectId>,
) -> Result<StatusCode> {
    store.project_delete(Scope::User(caller.user), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
