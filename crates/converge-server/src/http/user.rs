//! `/api/v1/users` — the list plus the caller-scoped `me`.
//!
//! `me` is a reserved identifier in the users collection (ULIDs can't spell
//! it). In single-user mode it resolves to the deployment's configured user
//! (`[user]` in config), ensured on read — deterministic by natural key, so
//! the row exists from the first call. Real auth replaces this resolution
//! with the authenticated principal.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use converge_storage::{Identity, Page, Pagination, Storage, StoreError, User, UserId};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<(S, Identity)> {
    Router::new()
        .route("/api/v1/users", get(list::<S>))
        .route("/api/v1/users/me", get(me::<S>))
}

async fn list<S: Storage>(
    State((store, _)): State<(S, Identity)>,
    Query(page): Query<Pagination<UserId>>,
) -> Result<Json<Page<User>>> {
    let items = store.user_list(page.clone()).await?;
    Ok(Json(Page::new(items, &page, |u| u.id.to_string())))
}

async fn me<S: Storage>(State((store, me)): State<(S, Identity)>) -> Result<Json<User>> {
    let id = store.user_login(me).await?;
    Ok(Json(store.user_get(id).await?.ok_or(StoreError::NotFound)?))
}
