//! `/api/v1/decisions` — CRUD, the atomic edit batch, and the graph edges,
//! over the [`Decisions`] trait.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, Edges, GroupId, NewDecision, Page,
    Pagination, ProjectId, Storage, StoreError,
};
use serde_json::{Value, json};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/decisions", post(add::<S>).get(list::<S>))
        .route("/api/v1/decisions/{id}", get(fetch::<S>).patch(edit::<S>))
        .route("/api/v1/decisions/{id}/edges", get(edges::<S>))
        .route("/api/v1/projects/{id}/decisions", get(by_project::<S>))
        .route("/api/v1/groups/{id}/decisions", get(by_group::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Json(new): Json<NewDecision>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.decision_add(new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// `?q=` switches the list into ranked search: best match first, no
/// cursor (rank order has no stable resume point — narrow the query or
/// raise `limit` instead).
#[derive(serde::Deserialize)]
struct Q {
    q: Option<String>,
}

/// List, narrowed by the filter (`?project=&group=&status=`), paged by
/// `?limit=&cursor=` — or searched by `?q=` (websearch syntax; ranked,
/// unpaged). Status matches the *derived* status — `superseded` finds
/// decisions with inbound edges.
async fn list<S: Storage>(
    State(store): State<S>,
    Query(filter): Query<DecisionFilter>,
    Query(q): Query<Q>,
    Query(page): Query<Pagination<DecisionId>>,
) -> Result<Json<Page<Decision>>> {
    if let Some(query) = q.q.as_deref() {
        if page.cursor.is_some() {
            return Err(StoreError::Invalid(
                "search results are ranked, not paged — drop the cursor".into(),
            )
            .into());
        }
        let items = store.decision_search(query, filter, page.limit).await?;
        return Ok(Json(Page {
            items,
            next_cursor: None,
        }));
    }
    let items = store.decision_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
}

/// Read-only relation projection: one project's decision log (the canonical
/// form stays `/decisions?project=`). The bound parent must exist — an
/// unknown project is 404, not `[]`.
async fn by_project<S: Storage>(
    State(store): State<S>,
    Path(id): Path<ProjectId>,
    Query(mut filter): Query<DecisionFilter>,
    Query(page): Query<Pagination<DecisionId>>,
) -> Result<Json<Page<Decision>>> {
    if filter.project.is_some() || filter.group.is_some() {
        return Err(StoreError::Invalid(
            "project is bound by the path; drop the project/group query parameters".into(),
        )
        .into());
    }
    store.project_get(id).await?.ok_or(StoreError::NotFound)?;
    filter.project = Some(id);
    let items = store.decision_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
}

/// Read-only relation projection: the group-wide feed, spanning the group's
/// projects. `?project=` narrows *within* the group — a child axis, not a
/// re-bind, so it stays allowed (a project outside the group just yields
/// nothing). The bound group must exist — unknown is 404, not `[]`.
async fn by_group<S: Storage>(
    State(store): State<S>,
    Path(id): Path<GroupId>,
    Query(mut filter): Query<DecisionFilter>,
    Query(page): Query<Pagination<DecisionId>>,
) -> Result<Json<Page<Decision>>> {
    if filter.group.is_some() {
        return Err(StoreError::Invalid(
            "group is bound by the path; drop the query parameter".into(),
        )
        .into());
    }
    store.group_get(id).await?.ok_or(StoreError::NotFound)?;
    filter.group = Some(id);
    let items = store.decision_list(filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
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
