//! Groups — the top-level container: a team's shared space or one person's.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{GroupId, UserId};
use crate::{Pagination, Scope, StoreError};

/// Whether the group is a team space or a single person's space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    Shared,
    Personal,
}

/// A group — owns projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub description: Option<String>,
    pub kind: GroupKind,
    /// Exactly one owner — implicitly a member, manages membership.
    pub owner: UserId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The fields required to create a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewGroup {
    pub name: String,
    pub description: Option<String>,
    pub kind: GroupKind,
}

/// A single group edit operation. `kind` is fixed at creation — turning a
/// personal space into a shared one is a different (future) operation, not
/// a field write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupEdit {
    SetName(String),
    SetDescription(Option<String>),
}

/// Storage operations on groups.
///
/// Reads are scope-filtered (visibility = owner OR member); an invisible
/// group is `None`/absent, indistinguishable from nonexistent. `group_edit`
/// is owner-only.
pub trait Groups {
    /// Create a group owned by `owner` — the caller, not body data.
    fn group_add(
        &self,
        owner: UserId,
        new: NewGroup,
    ) -> impl Future<Output = Result<GroupId, StoreError>> + Send;

    fn group_get(
        &self,
        scope: Scope,
        id: GroupId,
    ) -> impl Future<Output = Result<Option<Group>, StoreError>> + Send;

    /// Visible groups, newest first.
    fn group_list(
        &self,
        scope: Scope,
        page: Pagination<GroupId>,
    ) -> impl Future<Output = Result<Vec<Group>, StoreError>> + Send;

    /// Owner-only: a visible non-owner caller gets `Invalid`, an
    /// outsider `NotFound`.
    fn group_edit(
        &self,
        scope: Scope,
        id: GroupId,
        edits: Vec<GroupEdit>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Delete the group and everything under it: projects, their
    /// decisions and sessions, its memberships. Owner-only, like
    /// [`Groups::group_edit`]. `Conflict` when a session here anchors
    /// evidence of a decision *outside* the group — an evidenced
    /// message is undeletable (the evidence table's design invariant).
    fn group_delete(
        &self,
        scope: Scope,
        id: GroupId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
