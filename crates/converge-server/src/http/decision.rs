//! `/api/v1/decisions` — CRUD, the graph edges and the anchors, over the
//! [`Decisions`] trait.
//!
//! A decision's own fields change through a merge patch on the item
//! (RFC 7386: a field left out is untouched, `null` clears one that can
//! be empty). Its edges do not: superseding another decision, citing a
//! message, cross-referencing — each is a relation between two things
//! that exist, so each has its own address and answers to PUT and
//! DELETE.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use converge_storage::{
    Alternative, Author, CodeAnchor, Decision, DecisionEdit, DecisionFilter, DecisionId,
    DecisionStatus, Edges, GroupId, MessageId, NewDecision, Page, Pagination, ProjectId, Scope,
    Storage, StoreError,
};
use serde::{Deserialize, Deserializer};
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
        .route(
            "/api/v1/decisions/{id}/supersedes/{other}",
            put(supersede::<S>).delete(unsupersede::<S>),
        )
        .route(
            "/api/v1/decisions/{id}/related/{other}",
            put(relate::<S>).delete(unrelate::<S>),
        )
        .route(
            "/api/v1/decisions/{id}/evidence/{message}",
            put(anchor::<S>).delete(unanchor::<S>),
        )
        // A code anchor has no id of its own — commit, path and lines
        // are its key — so it is posted to the collection and deleted
        // by that key rather than addressed as an item.
        .route(
            "/api/v1/decisions/{id}/code-evidence",
            post(cite::<S>).delete(uncite::<S>),
        )
        .route("/api/v1/projects/{id}/decisions", get(by_project::<S>))
        .route("/api/v1/groups/{id}/decisions", get(by_group::<S>))
}

async fn add<S: Storage + 'static>(
    State((store, expert)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Json(mut new): Json<NewDecision>,
) -> Result<(StatusCode, Json<Value>)> {
    // A decision has an author. A body that names nobody was written
    // by whoever is calling; one that names others is recording theirs.
    if new.authors.is_empty() {
        new.authors.push(Author::User(caller.user));
    }
    let id = store.decision_add(Scope::User(caller.user), new).await?;
    crate::metrics::decision_recorded("rest");
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

/// A decision's own fields. Absent leaves a field alone; `null` clears
/// one that can be empty, which is why the two nullable fields are a
/// double option — the outer says whether the caller mentioned it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default)]
    status: Option<DecisionStatus>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    context: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    consequences: Option<Option<String>>,
    /// The whole list, replaced. There is no adding one alternative.
    #[serde(default)]
    alternatives: Option<Vec<Alternative>>,
}

/// `null` is a value here, not an absence — without this, serde cannot
/// tell "clear the context" from "do not touch it".
fn nullable<'de, T, D>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::deserialize(de).map(Some)
}

impl Patch {
    fn edits(self) -> Vec<DecisionEdit> {
        let mut edits = Vec::new();
        if let Some(status) = self.status {
            edits.push(DecisionEdit::SetStatus(status));
        }
        if let Some(title) = self.title {
            edits.push(DecisionEdit::SetTitle(title));
        }
        if let Some(summary) = self.summary {
            edits.push(DecisionEdit::SetSummary(summary));
        }
        if let Some(context) = self.context {
            edits.push(DecisionEdit::SetContext(context));
        }
        if let Some(consequences) = self.consequences {
            edits.push(DecisionEdit::SetConsequences(consequences));
        }
        if let Some(alternatives) = self.alternatives {
            edits.push(DecisionEdit::SetAlternatives(alternatives));
        }
        edits
    }
}

async fn edit<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
    Json(patch): Json<Patch>,
) -> Result<StatusCode> {
    let edits = patch.edits();
    if edits.is_empty() {
        // Nothing named is nothing to do, and the caller still learns
        // the decision exists and is theirs to edit.
        store
            .decision_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    store
        .decision_edit(Scope::User(caller.user), id, edits)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The reason a cross-reference exists, the one edge that carries a
/// payload. An empty body is a link with no reason given.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Why {
    #[serde(default)]
    why: Option<String>,
}

async fn one<S: Storage>(
    store: &S,
    caller: &Caller,
    id: DecisionId,
    edit: DecisionEdit,
) -> Result<StatusCode> {
    store
        .decision_edit(Scope::User(caller.user), id, vec![edit])
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// This decision replaces that one.
async fn supersede<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, other)): Path<(DecisionId, DecisionId)>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::AddSupersedes(other)).await
}

async fn unsupersede<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, other)): Path<(DecisionId, DecisionId)>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::RemoveSupersedes(other)).await
}

/// These two are worth reading together; `why` says what for.
async fn relate<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, other)): Path<(DecisionId, DecisionId)>,
    body: Option<Json<Why>>,
) -> Result<StatusCode> {
    let why = body.and_then(|Json(w)| w.why);
    one(
        &store,
        &caller,
        id,
        DecisionEdit::AddRelated { to: other, why },
    )
    .await
}

async fn unrelate<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, other)): Path<(DecisionId, DecisionId)>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::RemoveRelated(other)).await
}

/// This recorded turn is one of the lines that decided it.
async fn anchor<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, message)): Path<(DecisionId, MessageId)>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::AddEvidence(message)).await
}

/// The key of a code anchor, as a query for the delete.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorKey {
    commit: String,
    path: String,
    /// `start-end`, the way the range reads in a citation.
    lines: String,
}

impl AnchorKey {
    fn range(&self) -> Result<(u32, u32)> {
        let (start, end) = self
            .lines
            .split_once('-')
            .ok_or_else(|| StoreError::Invalid("lines must read `start-end`".into()))?;
        let parse = |n: &str| {
            n.trim()
                .parse::<u32>()
                .map_err(|_| StoreError::Invalid("lines must be numbers".into()))
        };
        Ok((parse(start)?, parse(end)?))
    }
}

/// Anchor a committed range; the anchor arrives whole and validated.
async fn cite<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
    Json(anchor): Json<CodeAnchor>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::AddCodeEvidence(anchor)).await
}

async fn uncite<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
    Query(key): Query<AnchorKey>,
) -> Result<StatusCode> {
    let lines = key.range()?;
    one(
        &store,
        &caller,
        id,
        DecisionEdit::RemoveCodeEvidence {
            commit: key.commit,
            path: key.path,
            lines,
        },
    )
    .await
}

async fn unanchor<S: Storage>(
    State((store, _)): State<(S, Expert<S>)>,
    Extension(caller): Extension<Caller>,
    Path((id, message)): Path<(DecisionId, MessageId)>,
) -> Result<StatusCode> {
    one(&store, &caller, id, DecisionEdit::RemoveEvidence(message)).await
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
