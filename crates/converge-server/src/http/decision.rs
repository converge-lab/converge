//! `/api/v1/decisions` — CRUD, the atomic edit batch, and the graph edges,
//! over the [`Decisions`] trait.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, Edges, GroupId, NewDecision, Page,
    Pagination, ProjectId, Scope, Storage, StoreError,
};
use serde_json::{Value, json};

use super::error::Result;
use crate::auth::Caller;
use crate::expert::Expert;

/// Decision routes carry the expert beside the store: `add` fires the
/// signal-detection pass post-commit (the write never waits on it).
pub fn routes<S: Storage + 'static>() -> Router<(S, Expert<S>)> {
    Router::new()
        .route("/api/v1/decisions", post(add::<S>).get(list::<S>))
        .route("/api/v1/decisions/{id}", get(fetch::<S>).patch(edit::<S>))
        .route("/api/v1/decisions/{id}/edges", get(edges::<S>))
        .route("/api/v1/projects/{id}/decisions", get(by_project::<S>))
        .route("/api/v1/groups/{id}/decisions", get(by_group::<S>))
}

async fn add<S: Storage + 'static>(
    State((store, expert)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewDecision>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.decision_add(Scope::User(caller.user), new).await?;
    expert.detect(id);
    // Prevention: hand the author same-project near-matches in the write
    // response — the one moment "did you mean to supersede?" is cheap.
    let similar: Vec<Value> = expert
        .similar(id)
        .await
        .into_iter()
        .map(|(id, title)| json!({ "id": id, "title": title }))
        .collect();
    let mut body = json!({ "id": id });
    if !similar.is_empty() {
        body["similar"] = json!(similar);
    }
    Ok((StatusCode::CREATED, Json(body)))
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
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Query(filter): Query<DecisionFilter>,
    Query(q): Query<Q>,
    Query(page): Query<Pagination<DecisionId>>,
) -> Result<Json<Page<Decision>>> {
    let scope = Scope::User(caller.user);
    if let Some(query) = q.q.as_deref() {
        if page.cursor.is_some() {
            return Err(StoreError::Invalid(
                "search results are ranked, not paged — drop the cursor".into(),
            )
            .into());
        }
        let items = store
            .decision_search(scope, query, filter, page.limit)
            .await?;
        return Ok(Json(Page {
            items,
            next_cursor: None,
        }));
    }
    let items = store.decision_list(scope, filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
}

/// Read-only relation projection: one project's decision log (the canonical
/// form stays `/decisions?project=`). The bound parent must exist — an
/// unknown project is 404, not `[]`.
async fn by_project<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
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
    let scope = Scope::User(caller.user);
    store
        .project_get(scope, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    filter.project = Some(id);
    let items = store.decision_list(scope, filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
}

/// Read-only relation projection: the group-wide feed, spanning the group's
/// projects. `?project=` narrows *within* the group — a child axis, not a
/// re-bind, so it stays allowed (a project outside the group just yields
/// nothing). The bound group must exist — unknown is 404, not `[]`.
async fn by_group<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
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
    let scope = Scope::User(caller.user);
    store
        .group_get(scope, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    filter.group = Some(id);
    let items = store.decision_list(scope, filter, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |d| d.id.to_string())))
}

async fn fetch<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
) -> Result<Json<Decision>> {
    Ok(Json(
        store
            .decision_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}

async fn edit<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
    Json(edits): Json<Vec<DecisionEdit>>,
) -> Result<StatusCode> {
    store
        .decision_edit(Scope::User(caller.user), id, edits)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The direct graph neighbourhood of one decision, both directions.
async fn edges<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
) -> Result<Json<Edges>> {
    Ok(Json(
        store
            .decision_edges(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}
