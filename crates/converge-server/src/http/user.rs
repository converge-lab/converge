//! `/api/v1/users` — today only the caller-scoped `me`.
//!
//! `me` is a reserved identifier in the users collection (ULIDs can't spell
//! it). In single-user mode it resolves to the deployment's configured user
//! (`[user]` in config), ensured on read — deterministic by natural key, so
//! the row exists from the first call. Real auth replaces this resolution
//! with the authenticated principal.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use converge_storage::{NewUser, Storage, StoreError, User};

use super::error::Result;

pub fn routes<S: Storage + 'static>() -> Router<(S, NewUser)> {
    Router::new().route("/api/v1/users/me", get(me::<S>))
}

async fn me<S: Storage>(State((store, me)): State<(S, NewUser)>) -> Result<Json<User>> {
    let id = store.user_ensure(me).await?;
    Ok(Json(store.user_get(id).await?.ok_or(StoreError::NotFound)?))
}
