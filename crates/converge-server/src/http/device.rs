//! `/api/v1/device/{user_code}` — the browser half of the device grant
//! (RFC 8628 §3.3): the signed-in user reads what is asking to pair and
//! approves or denies it. Session-authed like every `/api/v1` route; the
//! polling half lives on the open OAuth surface (`/oauth/token`).

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use converge_storage::{DeviceGrant, Storage, StoreError};
use serde::Deserialize;

use super::error::Result;
use crate::auth::Caller;
use crate::oauth::normalize_user_code;

pub fn routes<S: Storage + 'static>() -> Router<S> {
    Router::new().route(
        "/api/v1/device/{user_code}",
        get(show::<S>).post(decide::<S>),
    )
}

/// What the pair screen shows before the user commits: which client is
/// asking, and how long the code stays valid.
async fn show<S: Storage>(
    State(store): State<S>,
    Extension(_caller): Extension<Caller>,
    Path(user_code): Path<String>,
) -> Result<Json<DeviceGrant>> {
    let grant = store
        .device_get(&normalize_user_code(&user_code))
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(Json(grant))
}

#[derive(Deserialize)]
pub struct Decision {
    pub approve: bool,
}

/// The verdict. Approving binds the grant to the caller — the device
/// becomes *them* on its next poll.
async fn decide<S: Storage>(
    State(store): State<S>,
    Extension(caller): Extension<Caller>,
    Path(user_code): Path<String>,
    Json(decision): Json<Decision>,
) -> Result<StatusCode> {
    store
        .device_decide(
            &normalize_user_code(&user_code),
            caller.user,
            decision.approve,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
