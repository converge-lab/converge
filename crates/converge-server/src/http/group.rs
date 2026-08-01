//! `/api/v1/groups` — CRUD over the [`Groups`] trait plus membership
//! management, caller-scoped: you see the groups you own or belong to,
//! nothing else exists.
//!
//! Members are added directly by **handle** (owner-only): signing in is
//! what creates a user, so onboarding is "sign in once, then be added" —
//! there is no pending-invite state. The handle resolution is the one
//! deliberately unscoped lookup (the owner doesn't share a group with
//! the person yet, or they wouldn't be adding them).

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use converge_storage::{
    Group, GroupEdit, GroupId, Member, NewGroup, Page, Pagination, Scope, Storage, StoreError,
    UserId,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::error::Result;
use crate::auth::Caller;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new()
        .route("/api/v1/groups", post(add::<S>).get(list::<S>))
        .route("/api/v1/groups/{id}", get(fetch::<S>).patch(edit::<S>))
        .route(
            "/api/v1/groups/{id}/members",
            post(member_add::<S>).get(members::<S>),
        )
        .route(
            "/api/v1/groups/{id}/members/{user}",
            delete(member_remove::<S>),
        )
}

async fn add<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewGroup>,
) -> Result<(StatusCode, Json<Value>)> {
    // The creator owns it — ownership comes from authentication, never
    // from body data.
    let id = store.group_add(caller.user, new).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn list<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Query(page): Query<Pagination<GroupId>>,
) -> Result<Json<Page<Group>>> {
    let items = store
        .group_list(Scope::User(caller.user), page.clone())
        .await?;
    Ok(Json(Page::new(items, &page, |g| g.id.to_string())))
}

async fn fetch<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<GroupId>,
) -> Result<Json<Group>> {
    Ok(Json(
        store
            .group_get(Scope::User(caller.user), id)
            .await?
            .ok_or(StoreError::NotFound)?,
    ))
}

async fn edit<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<GroupId>,
    Json(edits): Json<Vec<GroupEdit>>,
) -> Result<StatusCode> {
    store
        .group_edit(Scope::User(caller.user), id, edits)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Who to add: an existing user, by handle (the human-friendly form) or
/// by id (the unambiguous one). Exactly one of the two.
#[derive(Deserialize)]
struct NewMember {
    handle: Option<String>,
    user_id: Option<UserId>,
}

async fn member_add<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<GroupId>,
    Json(new): Json<NewMember>,
) -> Result<StatusCode> {
    let user = match (new.user_id, new.handle.as_deref()) {
        (Some(user), None) => user,
        (None, Some(handle)) => {
            let mut found = store.user_lookup(handle).await?;
            match found.len() {
                0 => {
                    return Err(StoreError::Invalid(format!(
                        "no user with handle `{handle}` — they need to sign in once first"
                    ))
                    .into());
                }
                1 => found.remove(0).id,
                _ => {
                    return Err(StoreError::Invalid(format!(
                        "handle `{handle}` is ambiguous — add by user_id instead"
                    ))
                    .into());
                }
            }
        }
        _ => {
            return Err(
                StoreError::Invalid("provide exactly one of handle or user_id".into()).into(),
            );
        }
    };
    store.member_add(Scope::User(caller.user), id, user).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn members<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<GroupId>,
) -> Result<Json<Vec<Member>>> {
    Ok(Json(store.member_list(Scope::User(caller.user), id).await?))
}

async fn member_remove<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path((id, user)): Path<(GroupId, UserId)>,
) -> Result<StatusCode> {
    store
        .member_remove(Scope::User(caller.user), id, user)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
