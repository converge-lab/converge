//! `/api/v1/users` — the list plus the caller-scoped `me`.
//!
//! `me` is a reserved identifier in the users collection (ULIDs can't
//! spell it). It resolves to the **authenticated principal** — whoever the
//! presented token belongs to; the bootstrap flow guarantees the
//! deployment user exists before the first request.

use axum::Extension;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use converge_storage::{Page, Pagination, Storage, StoreError, User, UserId};

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/users", get(list::<S>))
        .route("/api/v1/users/me", get(me::<S>))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Query(page): Query<Pagination<UserId>>,
) -> Result<Json<Page<User>>> {
    let items = store.user_list(page.clone()).await?;
    Ok(Json(Page::new(items, &page, |u| u.id.to_string())))
}

async fn me<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<User>> {
    Ok(Json(
        store
            .user_get(caller.user)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}
