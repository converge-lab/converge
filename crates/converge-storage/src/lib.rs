//! The storage seam for Converge — the **Repository trait** the product is
//! written against.
//!
//! The domain layer, HTTP/MCP server, CLI, and web sit *above* this trait;
//! storage sits *below* it. Keeping the seam here makes the backend pluggable
//! and the domain testable against a fake. The bundled backend is
//! `converge-storage-postgres` (PostgreSQL).
//!
//! Methods follow a `resource_operation` naming (`decision_add`,
//! `decision_get`, …) so every resource's operations group together as the
//! trait grows. Edits are applied as an atomic batch of `*Edit` operations.

use std::future::Future;

use thiserror::Error;

pub mod decision;
pub mod ids;

pub use decision::{
    Alternative, Author, Decision, DecisionEdit, DecisionFilter, DecisionStatus, Edges,
    NewDecision, Related,
};
pub use ids::{AgentId, DecisionId, GroupId, ProjectId, UserId};

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

/// The domain storage trait. One method per domain operation, each backend
/// realizing it natively. Grows one record type at a time — decisions first.
///
/// (Spelled `-> impl Future + Send` so the returned futures are `Send`.)
pub trait Storage: Clone + Send + Sync {
    // ── decisions ───────────────────────────────────────────────────────
    fn decision_add(
        &self,
        new: NewDecision,
    ) -> impl Future<Output = Result<DecisionId, StoreError>> + Send;

    fn decision_get(
        &self,
        id: DecisionId,
    ) -> impl Future<Output = Result<Option<Decision>, StoreError>> + Send;

    fn decision_list(
        &self,
        filter: DecisionFilter,
    ) -> impl Future<Output = Result<Vec<Decision>, StoreError>> + Send;

    fn decision_edit(
        &self,
        id: DecisionId,
        edits: Vec<DecisionEdit>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The direct graph edges of `id`, or `None` when it doesn't exist.
    fn decision_edges(
        &self,
        id: DecisionId,
    ) -> impl Future<Output = Result<Option<Edges>, StoreError>> + Send;
}
