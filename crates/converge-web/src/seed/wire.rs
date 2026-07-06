//! Wire types — the JSON shapes the API serves and the client deserializes.
//!
//! Schema-backed resources are top-level; everything still awaiting tables
//! lives in [`mock`]. The wire `Decision` embeds its decision-owned relations
//! (authors, edges) and carries *derived* reverse edges (`superseded_by`,
//! `related_to`) computed by `assemble()` — never stored in the seed (D5).

use crate::seed::enums::{AgentKind, GroupKind, Status};
use serde::{Deserialize, Serialize};

/// A rejected alternative + why it lost (`decisions.alternatives` jsonb item).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

/// Group with its membership read-model: `project_ids` comes from the
/// `group_projects` relation so the client needs no extra join (D3).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: GroupKind,
    pub created_at: String,
    pub project_ids: Vec<String>,
}

/// Project (`projects` row, as served).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

/// User (`users` row, as served). `handle` is the natural key (a login /
/// username); `name` is display only — mirrors the upstream storage model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
    pub name: String,
}

/// Agent (`agents` row, as served).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub kind: AgentKind,
    pub name: String,
}

/// Author reference — mirrors the `decision_author` row. The three valid
/// states follow the upstream `Author` enum: user only (a person), agent only
/// (an autonomous agent), or both (a person working *through* an agent).
/// Neither-set is invalid (rejected by `validate()`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthorRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// A related-decision reference with the reason it matters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelatedRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// The wire decision: row fields + embedded relations + derived reverse edges.
///
/// Edge naming follows the upstream `Edges` struct: `related_to` = outgoing
/// cross-refs, `related_by` = incoming. `status` is as *read*: a decision with
/// any inbound supersedes-edge serves as `superseded` regardless of its stored
/// status (derived by `assemble()`, matching the upstream storage semantics).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub project_id: String,
    pub status: Status,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    #[serde(default)]
    pub authors: Vec<AuthorRef>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// Derived: decisions that supersede this one.
    #[serde(default)]
    pub superseded_by: Vec<String>,
    /// Outgoing cross-refs.
    #[serde(default)]
    pub related_to: Vec<RelatedRef>,
    /// Derived: incoming cross-refs (decisions that reference this one).
    #[serde(default)]
    pub related_by: Vec<RelatedRef>,
    pub captured_at: String,
}

/// Not-yet-relational shapes, served under `/mock/*` (D4).
pub mod mock {
    use crate::seed::enums::{Risk, SourceKind};
    use serde::{Deserialize, Serialize};

    /// `mock.me` as stored in the seed: resolved into [`Me`] at serve time.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct MeSeed {
        pub user_id: String,
        pub initial: String,
        pub role: String,
        pub email: String,
    }

    /// `GET /me` — the auth story; name/color resolved from users+user_colors.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Me {
        pub user_id: String,
        pub name: String,
        pub initial: String,
        pub role: String,
        pub email: String,
        pub color: String,
    }

    /// A cross-project signal (current shape, typed).
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Signal {
        pub id: String,
        pub from: String,
        /// Display text, not necessarily a project id (e.g. "all 6 services").
        pub to: String,
        pub dec_id: String,
        pub title: String,
        pub text: String,
        pub consequence: String,
        pub recommended: String,
        pub risk: Risk,
        #[serde(default)]
        pub sources: Vec<String>,
    }

    /// One line of an anchored source conversation.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct SourceLine {
        pub speaker: String,
        pub text: String,
        #[serde(default)]
        pub hl: bool,
    }

    /// Anchored evidence a decision was derived from.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Source {
        pub kind: SourceKind,
        pub label: String,
        pub when: String,
        #[serde(default)]
        pub lines: Vec<SourceLine>,
    }

    /// Per-decision data with no schema home yet (D8).
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Extras {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub session: Option<String>,
        #[serde(default)]
        pub tags: Vec<String>,
        #[serde(default)]
        pub sources: Vec<Source>,
    }
}
