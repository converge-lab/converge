//! The user — a person; one of the two author kinds.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::{Pagination, StoreError};

/// A person. `handle` is the natural key — how callers name the same person
/// across calls (a login, a username); `name` is display only. Provider
/// identity (OAuth uid etc.) is the auth layer's concern, layered on later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub handle: String,
    pub name: String,
}

/// The fields required to ensure a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewUser {
    pub handle: String,
    pub name: String,
}

/// Storage operations on users.
pub trait Users {
    /// Create-if-absent by `handle` — deterministic and race-safe (a single
    /// upsert, never scan-then-create). An existing user wins: `name` is
    /// stored only on first creation.
    fn user_ensure(&self, new: NewUser) -> impl Future<Output = Result<UserId, StoreError>> + Send;

    fn user_get(&self, id: UserId)
    -> impl Future<Output = Result<Option<User>, StoreError>> + Send;

    /// Users, newest first.
    fn user_list(
        &self,
        page: Pagination<UserId>,
    ) -> impl Future<Output = Result<Vec<User>, StoreError>> + Send;
}
