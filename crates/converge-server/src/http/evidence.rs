//! `/api/v1/sessions` and the evidence projections — the conversation
//! streams decisions cite, over the [`Sessions`]/[`Messages`] traits.
//!
//! `POST /sessions` is an **ensure** (the `(kind, external)` natural key
//! decides identity), so calling it twice converges on one id — importers
//! and live agents race on the same conversation by design. The message
//! stream is append-only and reads **oldest first** with a forward cursor
//! — the one list in the API that isn't newest-first. A decision's cited
//! excerpts live at `/decisions/{id}/sources`.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use converge_storage::{
    DecisionId, Message, MessageId, NewMessage, NewSession, Page, Pagination, Scope, Session,
    SessionFilter, SessionId, Source, Storage, StoreError,
};
use serde_json::{Value, json};

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/sessions", post(ensure::<S>).get(list::<S>))
        .route("/api/v1/sessions/{id}", get(fetch::<S>))
        .route(
            "/api/v1/sessions/{id}/messages",
            post(append::<S>).get(stream::<S>),
        )
        .route("/api/v1/decisions/{id}/sources", get(sources::<S>))
}

/// Create-or-refresh by `(kind, external)`; answers 201 with the id either
/// way — the caller asked for the session to exist, and now it does.
async fn ensure<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewSession>,
) -> Result<(StatusCode, Json<Value>)> {
    let id = store.session_ensure(Scope::User(caller.user), new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Query(filter): Query<SessionFilter>,
    Query(page): Query<Pagination<SessionId>>,
) -> Result<Json<Page<Session>>> {
    let items = store
        .session_list(Scope::User(caller.user), filter, page.clone())
        .await?;
    Ok(Json(Page::new(items, &page, |s| s.id.to_string())))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    Ok(Json(
        store
            .session_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}

/// Append a batch to the stream; answers the new message ids, in order.
async fn append<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<SessionId>,
    Json(new): Json<Vec<NewMessage>>,
) -> Result<(StatusCode, Json<Value>)> {
    let ids = store.message_add(Scope::User(caller.user), id, new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "ids": ids }))))
}

/// The stream, oldest first; `?cursor=` returns messages strictly after
/// it. The bound session must exist — unknown is 404, not `[]`.
async fn stream<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<SessionId>,
    Query(page): Query<Pagination<MessageId>>,
) -> Result<Json<Page<Message>>> {
    let scope = Scope::User(caller.user);
    store
        .session_get(scope, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let items = store.message_list(scope, id, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |m| m.id.to_string())))
}

/// A decision's cited excerpts: sessions with anchors + context, derived
/// at read time from the stored anchors.
async fn sources<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<DecisionId>,
) -> Result<Json<Vec<Source>>> {
    Ok(Json(
        store
            .decision_sources(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}
