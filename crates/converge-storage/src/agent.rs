//! The agent — an autonomous actor; the other author kind.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::ids::AgentId;
use crate::{Pagination, StoreError};

/// What kind of actor an agent is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    /// An LLM (e.g. a Claude model).
    Model,
    /// A non-model program (a CLI, a bot, a pipeline).
    Tool,
}

/// An autonomous actor. `(kind, name)` is the natural key — agents
/// self-report on every call, so the same actor must resolve to the same
/// record deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub kind: AgentKind,
    pub name: String,
}

/// The fields required to ensure an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAgent {
    pub kind: AgentKind,
    pub name: String,
}

/// Storage operations on agents.
pub trait Agents {
    /// Create-if-absent by `(kind, name)` — deterministic and race-safe (a
    /// single upsert, never scan-then-create).
    fn agent_ensure(
        &self,
        new: NewAgent,
    ) -> impl Future<Output = Result<AgentId, StoreError>> + Send;

    fn agent_get(
        &self,
        id: AgentId,
    ) -> impl Future<Output = Result<Option<Agent>, StoreError>> + Send;

    /// Agents, newest first.
    fn agent_list(
        &self,
        page: Pagination<AgentId>,
    ) -> impl Future<Output = Result<Vec<Agent>, StoreError>> + Send;
}
