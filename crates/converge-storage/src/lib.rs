//! The storage seam for Converge — the **Repository traits** the product is
//! written against.
//!
//! The domain layer, HTTP/MCP server, CLI, and web sit *above* these traits;
//! storage sits *below* them. Keeping the seam here makes the backend
//! pluggable and the domain testable against a fake. The bundled backend is
//! `converge-storage-postgres` (PostgreSQL).
//!
//! Each resource module carries its types **and** its storage trait
//! ([`Groups`], [`Projects`], [`Decisions`], …); [`Storage`] bundles them
//! all. Consumers narrow to the trait they need (`fn f<S: Decisions>(…)`) or
//! take the whole surface (`S: Storage`); backends implement the per-resource
//! traits and get `Storage` from the blanket impl.
//!
//! Methods follow a `resource_operation` naming (`decision_add`,
//! `decision_get`, …): the bundle merges every trait into one method
//! namespace, so the resource prefix is what keeps names collision-free.
//! Edits are applied as an atomic batch of `*Edit` operations.
//! (Methods are spelled `-> impl Future + Send` so the returned futures are
//! `Send`.)

use thiserror::Error;

pub mod decision;
pub mod group;
pub mod ids;
pub mod project;

pub use decision::{
    Alternative, Author, Decision, DecisionEdit, DecisionFilter, DecisionStatus, Decisions, Edges,
    NewDecision, Related,
};
pub use group::{Group, GroupEdit, GroupKind, Groups, NewGroup};
pub use ids::{AgentId, DecisionId, GroupId, ProjectId, UserId};
pub use project::{NewProject, Project, ProjectEdit, ProjectFilter, Projects};

/// Backend-agnostic storage error. A backend maps its native failures into
/// these; callers distinguish only what they need to act on.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    /// Backend unreachable / transient — retryable, distinct from a logical failure.
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("backend error: {0}")]
    Backend(String),
}

/// The full storage surface — every resource trait, bundled. Implemented
/// automatically for any type that implements them all.
pub trait Storage: Groups + Projects + Decisions + Clone + Send + Sync {}

impl<T: Groups + Projects + Decisions + Clone + Send + Sync> Storage for T {}
