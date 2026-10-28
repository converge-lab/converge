//! Seed row types — a 1:1 mirror of the SQL tables (§ `migrations/`), plus the
//! `mock` namespace for data that has no tables yet (D4/D8/D10).
//!
//! The demo dataset ships inside this crate as [`EMBEDDED`]; both the mock
//! server and the app's embedded source parse it through [`Seed::parse`], so
//! there is exactly one copy and one shape.

use crate::seed::enums::{AgentKind, GroupKind, Status};
use crate::seed::wire;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The demo dataset, embedded at compile time.
pub const EMBEDDED: &str = include_str!("seed.json");

/// `groups` row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupRow {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: GroupKind,
    pub created_at: String,
}

/// `projects` row. `group_id` is the *owning* group; full membership lives in
/// [`GroupProjectRow`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

/// `group_projects` row (membership, owner included).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupProjectRow {
    pub group_id: String,
    pub project_id: String,
}

/// `users` row. `handle` is the natural key; `name` is display only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserRow {
    pub id: String,
    pub handle: String,
    pub name: String,
}

/// `agents` row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub kind: AgentKind,
    pub name: String,
}

/// `decisions` row. `alternatives` is the jsonb column, already typed.
///
/// `status` is the *stored* status and can never be `superseded` — that state
/// is derived from inbound supersedes-edges at assembly, mirroring the
/// upstream storage semantics (enforced by `validate()`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionRow {
    pub id: String,
    pub project_id: String,
    pub status: Status,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    #[serde(default)]
    pub alternatives: Vec<wire::Alternative>,
    pub captured_at: String,
}

/// `decision_author` row: at least one of `user_id`/`agent_id` (validated).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionAuthorRow {
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// `decision_supersedes` row (edge: decision → the one it supersedes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionSupersedesRow {
    pub decision_id: String,
    pub supersedes_id: String,
}

/// `decision_related` row (edge: decision → referenced decision, with why).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionRelatedRow {
    pub decision_id: String,
    pub ref_id: String,
    pub why: Option<String>,
}

/// Everything not yet relational (D4): served under `/mock/*`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MockData {
    pub me: wire::mock::MeSeed,
    #[serde(default)]
    pub user_colors: HashMap<String, String>,
    #[serde(default)]
    pub signals: Vec<wire::mock::Signal>,
    #[serde(default)]
    pub decision_extras: HashMap<String, wire::mock::Extras>,
    #[serde(default)]
    pub unread: Vec<String>,
    #[serde(default)]
    pub agent_context: HashMap<String, Vec<String>>,
}

/// The whole seed: table rows + mock namespace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Seed {
    pub groups: Vec<GroupRow>,
    pub projects: Vec<ProjectRow>,
    pub group_projects: Vec<GroupProjectRow>,
    pub users: Vec<UserRow>,
    pub agents: Vec<AgentRow>,
    pub decisions: Vec<DecisionRow>,
    pub decision_author: Vec<DecisionAuthorRow>,
    #[serde(default)]
    pub decision_supersedes: Vec<DecisionSupersedesRow>,
    #[serde(default)]
    pub decision_related: Vec<DecisionRelatedRow>,
    pub mock: MockData,
}

impl Seed {
    /// Parse a seed from JSON.
    pub fn parse(json: &str) -> Result<Seed, serde_json::Error> {
        serde_json::from_str(json)
    }
}
