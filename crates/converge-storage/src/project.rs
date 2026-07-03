//! Projects — a logical codebase/service, owned by a group.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::StoreError;
use crate::ids::{GroupId, ProjectId};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFilter {
    pub group: Option<GroupId>,
    pub limit: Option<u32>,
}

/// Storage operations on projects.
pub trait Projects {
    fn project_add(
        &self,
        new: NewProject,
    ) -> impl Future<Output = Result<ProjectId, StoreError>> + Send;

    fn project_get(
        &self,
        id: ProjectId,
    ) -> impl Future<Output = Result<Option<Project>, StoreError>> + Send;

    fn project_list(
        &self,
        filter: ProjectFilter,
    ) -> impl Future<Output = Result<Vec<Project>, StoreError>> + Send;

    fn project_edit(
        &self,
        id: ProjectId,
        edits: Vec<ProjectEdit>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
