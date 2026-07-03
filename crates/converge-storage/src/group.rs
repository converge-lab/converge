//! Groups — the top-level container: a team's shared space or one person's.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::StoreError;
use crate::ids::GroupId;

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
pub trait Groups {
    fn group_add(&self, new: NewGroup) -> impl Future<Output = Result<GroupId, StoreError>> + Send;

    fn group_get(
        &self,
        id: GroupId,
    ) -> impl Future<Output = Result<Option<Group>, StoreError>> + Send;

    /// All groups, newest first.
    fn group_list(&self) -> impl Future<Output = Result<Vec<Group>, StoreError>> + Send;

    fn group_edit(
        &self,
        id: GroupId,
        edits: Vec<GroupEdit>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
