//! `/api/v1/tokens` — the caller's bearer tokens (the GitHub-PAT UX).
//!
//! Always scoped to the authenticated caller: you list, create, and revoke
//! **your own** tokens, never anyone else's (someone else's id reads as
//! 404). `POST` answers with the secret exactly once; only its hash is
//! stored. Host-side administration (`converge-server token …`) rides the
//! same storage seam with the same semantics.

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use converge_storage::{Minted, NewToken, Page, Pagination, Storage, Token, TokenId};

use super::error::Result;
use crate::auth::{self, Caller};

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/tokens", post(add::<S>).get(list::<S>))
        .route("/api/v1/tokens/{id}", delete(revoke::<S>))
}

async fn add<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewToken>,
) -> Result<(StatusCode, Json<Minted>)> {
    let token = auth::mint();
    let id = store
        .token_add(caller.user, new.label, auth::hash(&token))
        .await?;
    Ok((StatusCode::CREATED, Json(Minted { id, token })))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Query(page): Query<Pagination<TokenId>>,
) -> Result<Json<Page<Token>>> {
    let items = store.token_list(caller.user, page.clone()).await?;
    Ok(Json(Page::new(items, &page, |t| t.id.to_string())))
}

async fn revoke<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<TokenId>,
) -> Result<StatusCode> {
    store.token_revoke(caller.user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
