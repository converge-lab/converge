//! The decision (ADR) — the core record and the node of the decision graph.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{AgentId, DecisionId, GroupId, ProjectId, UserId};

/// Lifecycle of a decision.
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
}

/// A single edit operation. Applied as a batch (`Vec<DecisionEdit>`)
/// atomically — sparse (only the ops you send) and race-safe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionEdit {
    SetStatus(DecisionStatus),
    SetTitle(String),
    SetSummary(String),
    SetContext(Option<String>),
    SetConsequences(Option<String>),
    SetAlternatives(Vec<Alternative>),
}

/// Filter for listing decisions. All fields optional; combine to narrow.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionFilter {
    pub project: Option<ProjectId>,
    pub group: Option<GroupId>,
    pub status: Option<DecisionStatus>,
    pub limit: Option<u32>,
}
