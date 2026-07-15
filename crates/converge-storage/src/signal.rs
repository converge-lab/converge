//! A signal is an observation that one decision affects other decisions.
//!
//! It is a typed `decision -> decisions` edge with a response tier, an open
//! relation kind, and a lifecycle. This module deliberately contains only
//! the domain data contract; persistence operations are introduced together
//! with the signal storage schema.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::decision::Author;
use crate::ids::{DecisionId, SignalId};

/// How urgently the affected side should react: the cost-of-inaction ladder,
/// lowest first.
///
/// This axis is fixed because product behavior can route on it. The
/// open-ended reason why the relationship exists is [`Signal::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Worth knowing; no action is currently required.
    Watch,
    /// Recoverable drift: the involved projects should coordinate.
    Coordinate,
    /// Acting on the source decision breaks the target decisions.
    WillBreak,
}

/// Lifecycle of a signal.
///
/// A signal is born as a proposal because it is an observation rather than an
/// established fact. It can later be validated or dismissed as false/stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    Proposed,
    Validated,
    Dismissed,
}

/// A signal as recorded by Converge.
///
/// Group and project membership are derived from the referenced decisions and
/// are therefore not duplicated on the signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    /// The decision that triggered the observation.
    pub source: DecisionId,
    /// Decisions affected by `source`. Must be non-empty and must not contain
    /// `source`.
    pub targets: Vec<DecisionId>,
    pub risk: Risk,
    /// Open relation label such as `dependency`, `duplication`, or
    /// `divergence`.
    pub kind: String,
    pub status: SignalStatus,
    pub title: String,
    /// What is happening.
    pub text: String,
    /// What happens if the signal is ignored.
    pub consequence: String,
    /// What the involved projects should do.
    pub recommendation: String,
    pub produced_by: Author,
    /// Present exactly when the signal has been validated.
    pub validated_by: Option<Author>,
    /// When Converge recorded the signal.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A request to record a new signal.
///
/// It intentionally carries neither an id, lifecycle status, author, nor
/// timestamp. The domain layer validates the references and stamps those
/// fields when the proposal is persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalCreate {
    pub source: DecisionId,
    pub targets: Vec<DecisionId>,
    pub risk: Risk,
    pub kind: String,
    pub title: String,
    pub text: String,
    pub consequence: String,
    pub recommendation: String,
}
