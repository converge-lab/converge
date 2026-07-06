//! Wire enums. These mirror the SQL `check` constraints; serde speaks
//! `snake_case` strings on the wire (`"will_break"`, `"accepted"`, …).

use serde::{Deserialize, Serialize};

/// Lifecycle of a decision (`decisions.status`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Accepted,
    Draft,
    Proposed,
    Superseded,
    Rejected,
}

/// `groups.kind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Shared,
    Personal,
}

/// `agents.kind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Model,
    Tool,
}

/// Severity of a cross-project signal (mock namespace; no table yet).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    WillBreak,
    Coordinate,
    Watch,
}

/// Kind of anchored evidence (mock namespace; no table yet).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Transcript,
    Slack,
    Pr,
    Incident,
}
