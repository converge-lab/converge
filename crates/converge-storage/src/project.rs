//! Projects — a logical codebase/service, owned by a group.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{GroupId, ProjectId};
use crate::{Pagination, Scope, StoreError};

/// Where a project's code lives. The host is a variant because the host
/// is what an integration keys on; a code evidence anchor never depends
/// on it — a commit sha is the same in every clone, so mirrors and forks
/// need nothing, and only links and integrations use the canonical name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repository {
    /// Any git host, normalized: scheme, credentials and `.git` stripped,
    /// host lower-cased — `example.com/team/repo`.
    Git { url: String },
    /// github.com, resolvable and validatable once the App is installed.
    Github { owner: String, name: String },
}

impl Repository {
    /// From a remote as git reports it: `git@github.com:o/n.git`,
    /// `https://github.com/o/n`, `ssh://git@host/team/repo.git`. Nothing
    /// recognisable (no host and path) is `None`.
    pub fn from_remote(remote: &str) -> Option<Self> {
        let remote = remote.trim();
        if remote.is_empty() {
            return None;
        }
        // A URL keeps `host[:port]` up to the first slash; the scp-like
        // `user@host:path` form has no port and splits at the colon.
        // Both lose their credentials.
        let (url, rest) = match remote.split_once("://") {
            Some((_, rest)) => (true, rest),
            None => (false, remote),
        };
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let split = if url {
            rest.split_once('/')
        } else {
            rest.split_once(':').or_else(|| rest.split_once('/'))
        };
        let (host, path) = split?;
        let host = host.to_lowercase();
        let path = path
            .trim_matches('/')
            .trim_end_matches(".git")
            .trim_end_matches('/');
        if host.is_empty() || path.is_empty() {
            return None;
        }
        if host == "github.com"
            && let Some((owner, name)) = path.split_once('/')
            && !owner.is_empty()
            && !name.is_empty()
            && !name.contains('/')
        {
            return Some(Repository::Github {
                owner: owner.to_owned(),
                name: name.to_owned(),
            });
        }
        Some(Repository::Git {
            url: format!("{host}/{path}"),
        })
    }

    /// The canonical name: `github.com/owner/name`, or the normalized url.
    pub fn canonical(&self) -> String {
        match self {
            Repository::Git { url } => url.clone(),
            Repository::Github { owner, name } => format!("github.com/{owner}/{name}"),
        }
    }
}

/// A project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub group_id: GroupId,
    /// Display name only — identity is the id.
    pub name: String,
    pub description: Option<String>,
    /// Where its code lives; set the first time a hook binds it, from
    /// the remote the hook sends, and editable afterwards.
    #[serde(default)]
    pub repository: Option<Repository>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The fields required to create a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub group_id: GroupId,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<Repository>,
}

/// A single project edit operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEdit {
    SetName(String),
    SetDescription(Option<String>),
    SetRepository(Option<Repository>),
}

#[cfg(test)]
mod tests {
    use super::Repository;

    #[test]
    fn remotes_normalize_to_one_repository() {
        let github = Repository::Github {
            owner: "converge-lab".into(),
            name: "converge".into(),
        };
        for remote in [
            "git@github.com:converge-lab/converge.git",
            "https://github.com/converge-lab/converge",
            "https://user:token@GitHub.com/converge-lab/converge.git/",
            "ssh://git@github.com/converge-lab/converge.git",
        ] {
            assert_eq!(
                Repository::from_remote(remote),
                Some(github.clone()),
                "{remote}"
            );
        }
        assert_eq!(github.canonical(), "github.com/converge-lab/converge");
        assert_eq!(
            Repository::from_remote("ssh://git@gitlab.example.com:2222/team/sub/repo.git"),
            Some(Repository::Git {
                url: "gitlab.example.com:2222/team/sub/repo".into()
            })
        );
        assert_eq!(
            Repository::from_remote("git@example.com:team/repo"),
            Some(Repository::Git {
                url: "example.com/team/repo".into()
            })
        );
        assert_eq!(Repository::from_remote(""), None);
        assert_eq!(Repository::from_remote("just-a-name"), None);
    }
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

    /// Delete the project and everything under it: decisions (with
    /// their edges, signals, evidence) and sessions (with their
    /// messages). Owner-of-the-owning-group only. `Conflict` when one
    /// of its sessions anchors evidence of a surviving decision in
    /// another project — an evidenced message is undeletable.
    fn project_delete(
        &self,
        scope: Scope,
        id: ProjectId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
