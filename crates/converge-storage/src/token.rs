//! Bearer tokens — how non-browser callers (agents, the CLI) authenticate.
//!
//! Storage only ever sees the **hash** of a token: the secret is generated,
//! shown once, and hashed by the server; a database leak leaks no usable
//! credentials. Lookup is by hash equality — tokens are high-entropy
//! random, so an unsalted digest is the standard construction (same as
//! GitHub's).

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{TokenId, UserId};
use crate::{Pagination, StoreError};

/// A token record, as listed. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub id: TokenId,
    pub user_id: UserId,
    /// What this token is for ("bootstrap admin", "laptop CLI", …).
    pub label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Storage operations on tokens.
pub trait Tokens {
    /// Store a new token's hash for `user`.
    fn token_add(
        &self,
        user: UserId,
        label: String,
        hash: String,
    ) -> impl Future<Output = Result<TokenId, StoreError>> + Send;

    /// Resolve a presented token (by its hash) to the owning user — the
    /// authentication lookup. `None` is "no such token".
    fn token_user(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<UserId>, StoreError>> + Send;

    /// One user's tokens, newest first.
    fn token_list(
        &self,
        user: UserId,
        page: Pagination<TokenId>,
    ) -> impl Future<Output = Result<Vec<Token>, StoreError>> + Send;
}
