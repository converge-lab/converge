//! Wire enums. These mirror the SQL `check` constraints; serde speaks
//! `snake_case` strings on the wire (`"conflict"`, `"accepted"`, …).

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

/// Severity of a cross-project signal (the server's `conflict` tier
/// renders as `Conflict`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Conflict,
    Coordinate,
    Watch,
}

/// Lifecycle of a signal: born proposed, judged into confirmed or
/// dismissed. Defaults to proposed so the fixture seed stays terse.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    #[default]
    Proposed,
    Confirmed,
    Dismissed,
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
