//! The user — a person; one of the two author kinds.
//!
//! Identity is `(provider, subject)` — the immutable pair an auth provider
//! asserts (e.g. `("github", "<numeric id>")`, or `("local", <handle>)` for
//! the deployment's bootstrap user). `handle` is a login/username and
//! **mutable**: providers let people rename, so every login refreshes it.
//! Never key on a handle; key on identity. (Contrast with agents, where
//! `(kind, name)` *is* the identity and the first write wins.)

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::{Pagination, StoreError};

/// A person, as stored and served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    /// The asserting auth provider (`github`, `local`, …).
    pub provider: String,
    /// The provider's immutable id for this person.
    pub subject: String,
    /// Login/username — display and mention material, refreshed on login.
    pub handle: String,
    /// Display name, refreshed on login.
    pub name: String,
}

/// A login assertion from an auth provider (or the deployment config, as
/// provider `local`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: String,
    pub subject: String,
    pub handle: String,
    pub name: String,
}

/// Storage operations on users.
pub trait Users {
    /// Create-or-refresh by identity — deterministic and race-safe (a
    /// single upsert). The `(provider, subject)` pair decides *who*; the
    /// mutable fields (`handle`, `name`) are refreshed on every login, so
    /// renames on the provider side propagate.
    fn user_login(
        &self,
        identity: Identity,
    ) -> impl Future<Output = Result<UserId, StoreError>> + Send;

    fn user_get(&self, id: UserId)
    -> impl Future<Output = Result<Option<User>, StoreError>> + Send;

    /// Users, newest first.
    fn user_list(
        &self,
        page: Pagination<UserId>,
    ) -> impl Future<Output = Result<Vec<User>, StoreError>> + Send;
}
