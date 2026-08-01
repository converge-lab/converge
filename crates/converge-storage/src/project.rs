//! Projects — a logical codebase/service, owned by a group.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{GroupId, ProjectId};
use crate::{Pagination, Scope, StoreError};

/// A project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub group_id: GroupId,
    /// Display name only — identity is the id.
    pub name: String,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The fields required to create a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub group_id: GroupId,
    pub name: String,
    pub description: Option<String>,
}

/// A single project edit operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEdit {
    SetName(String),
    SetDescription(Option<String>),
}

/// Filter for listing projects. All fields optional; combine to narrow.
/// Pagination travels separately ([`Pagination`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFilter {
    pub group: Option<GroupId>,
}

/// Storage operations on projects. Reads are scope-filtered through the
/// owning group; writes require the target group to be visible (an
/// invisible group is `NotFound`).
pub trait Projects {
    fn project_add(
        &self,
        scope: Scope,
        new: NewProject,
    ) -> impl Future<Output = Result<ProjectId, StoreError>> + Send;

    fn project_get(
        &self,
        scope: Scope,
        id: ProjectId,
    ) -> impl Future<Output = Result<Option<Project>, StoreError>> + Send;

    fn project_list(
        &self,
        scope: Scope,
        filter: ProjectFilter,
        page: Pagination<ProjectId>,
    ) -> impl Future<Output = Result<Vec<Project>, StoreError>> + Send;

    fn project_edit(
        &self,
        scope: Scope,
        id: ProjectId,
        edits: Vec<ProjectEdit>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
