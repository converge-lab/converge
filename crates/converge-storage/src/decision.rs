//! The decision (ADR) — the core record and the node of the decision graph.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::StoreError;
use crate::ids::{AgentId, DecisionId, GroupId, ProjectId, UserId};

/// Lifecycle of a decision.
///
/// `Superseded` is **derived**: a decision with any inbound supersedes-edge
/// reads as superseded, whatever its stored status. It can't be stored or
/// set directly — supersede via edges; removing the last inbound edge
/// restores the stored status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionStatus {
    Accepted,
    Draft,
    Proposed,
    Superseded,
    Rejected,
}

/// Who authored a decision. The three valid states are the *only* states —
/// a "neither" author is unrepresentable. Maps to the `decision_author`
/// `(user_id?, agent_id?)` columns at the storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Author {
    /// Created directly by a person (e.g. in the UI).
    User(UserId),
    /// Produced autonomously by an agent.
    Agent(AgentId),
    /// A person working through an agent.
    UserViaAgent { user: UserId, agent: AgentId },
}

/// A rejected alternative and why it lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

/// A decision record — core fields plus its authors. Graph edges (chain,
/// cross-refs, signals) and evidence are separate reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub project_id: ProjectId,
    pub status: DecisionStatus,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    pub alternatives: Vec<Alternative>,
    pub authors: Vec<Author>,
    pub captured_at: OffsetDateTime,
}

/// The fields required to create a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewDecision {
    pub project_id: ProjectId,
    pub status: DecisionStatus,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    pub alternatives: Vec<Alternative>,
    pub authors: Vec<Author>,
    /// Decisions this one replaces — creation-time supersession edges.
    pub supersedes: Vec<DecisionId>,
}

/// A single edit operation. Applied as a batch (`Vec<DecisionEdit>`)
/// atomically — sparse (only the ops you send) and race-safe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionEdit {
    /// `SetStatus(Superseded)` is invalid — superseded is derived from edges.
    SetStatus(DecisionStatus),
    SetTitle(String),
    SetSummary(String),
    SetContext(Option<String>),
    SetConsequences(Option<String>),
    SetAlternatives(Vec<Alternative>),
    /// Add a supersession edge: this decision replaces the target.
    AddSupersedes(DecisionId),
    /// Remove a supersession edge (no-op when absent).
    RemoveSupersedes(DecisionId),
    /// Add a cross-reference; re-adding an existing one updates `why`.
    AddRelated { to: DecisionId, why: Option<String> },
    /// Remove a cross-reference (no-op when absent).
    RemoveRelated(DecisionId),
}

/// One end of a cross-reference edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Related {
    pub id: DecisionId,
    pub why: Option<String>,
}

/// The direct graph edges of one decision, both directions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edges {
    /// Decisions this one replaced.
    pub supersedes: Vec<DecisionId>,
    /// Decisions that replaced this one (non-empty ⇒ reads as superseded).
    pub superseded_by: Vec<DecisionId>,
    /// Outgoing cross-refs.
    pub related_to: Vec<Related>,
    /// Incoming cross-refs.
    pub related_by: Vec<Related>,
}

/// Filter for listing decisions. All fields optional; combine to narrow.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionFilter {
    pub project: Option<ProjectId>,
    pub group: Option<GroupId>,
    pub status: Option<DecisionStatus>,
    pub limit: Option<u32>,
}

/// Storage operations on decisions and their graph edges.
pub trait Decisions {
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
