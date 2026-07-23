//! The signal — a typed observation that one decision affects others:
//! "you added X; it bears on Y (and Z) — here is how and what to do."
//!
//! One signal is **one observation**: a single source decision, one open
//! relation `kind`, a response tier, one narrative, one lifecycle. The
//! decisions it affects are a target *set* (`signal_targets` at the
//! storage layer) — the graph reads project each `(source, target)` pair
//! as an edge, while dismissal and confirmation apply to the observation
//! as a whole (an observation is wrong or stale in one piece).
//!
//! Signals are born `proposed` — they are observations, not facts — and
//! resolve to `confirmed` (evidence held up) or `dismissed` (wrong, or
//! not worth acting on). Dismissed signals stay: they are the "don't
//! raise this again" memory, and storage enforces it — a new signal
//! repeating any `(source, target, kind)` pair of an existing one, in
//! any status, is a conflict.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::decision::Author;
use crate::ids::{DecisionId, ProjectId, SignalId};
use crate::{Pagination, StoreError};

/// The response tier — how urgently the affected side should react; the
/// severity-of-inaction ladder, lowest first. Fixed (responses are
/// bounded: absorb / negotiate / resolve) while `kind` stays open; the
/// tier routes, a future relevance score sorts within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Worth knowing; no action required.
    Watch,
    /// Recoverable drift or a dependency — the projects should talk.
    Coordinate,
    /// An incompatibility: acting on the source breaks the targets.
    Conflict,
}

/// Lifecycle of a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalStatus {
    /// An observation awaiting judgment — every signal starts here.
    Proposed,
    /// Held up under scrutiny (`resolved_by` says whose).
    Confirmed,
    /// Wrong, stale, or not worth acting on — kept as the don't-re-raise
    /// memory.
    Dismissed,
}

/// A signal, as stored and served.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    /// The decision that triggered the observation. For symmetric kinds
    /// (duplication, overlap) this is the *discovery* direction: the
    /// newcomer points at what already existed.
    pub source: DecisionId,
    /// The decisions it bears on — a non-empty set, never containing
    /// `source`. Stable but unspecified order.
    pub targets: Vec<DecisionId>,
    /// Open relation label (`dependency`, `duplication`, `divergence`, …)
    /// — why the relationship exists, snake_case by convention.
    pub kind: String,
    pub tier: Tier,
    pub status: SignalStatus,
    pub title: String,
    /// What is happening (Markdown).
    pub text: String,
    /// The cost of ignoring it.
    pub consequence: Option<String>,
    /// What the involved projects should do.
    pub recommendation: Option<String>,
    pub produced_by: Author,
    /// Who confirmed or dismissed it — present exactly when resolved.
    pub resolved_by: Option<Author>,
    /// When converge recorded it (server-assigned).
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

/// The fields required to record a signal. Status is not among them —
/// every signal is born `proposed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSignal {
    pub source: DecisionId,
    pub targets: Vec<DecisionId>,
    pub kind: String,
    pub tier: Tier,
    pub title: String,
    pub text: String,
    pub consequence: Option<String>,
    pub recommendation: Option<String>,
    pub produced_by: Author,
}

/// Narrowing for signal lists; fields compose (AND).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalFilter {
    /// Signals touching this project on either end (its decisions as
    /// source or target).
    pub project: Option<ProjectId>,
    /// Signals touching this decision on either end.
    pub decision: Option<DecisionId>,
    pub status: Option<SignalStatus>,
    pub tier: Option<Tier>,
}

/// Storage operations on signals.
pub trait Signals {
    /// Record an observation. Targets collapse to a set; empty targets,
    /// a target equal to `source`, an empty `kind`, or an unknown
    /// decision are `Invalid`/`NotFound`. Repeating any existing
    /// `(source, target, kind)` pair — whatever that signal's status —
    /// is a `Conflict`: dismissed observations are not re-raised.
    fn signal_add(
        &self,
        new: NewSignal,
    ) -> impl Future<Output = Result<SignalId, StoreError>> + Send;

    fn signal_get(
        &self,
        id: SignalId,
    ) -> impl Future<Output = Result<Option<Signal>, StoreError>> + Send;

    /// Signals, newest first.
    fn signal_list(
        &self,
        filter: SignalFilter,
        page: Pagination<SignalId>,
    ) -> impl Future<Output = Result<Vec<Signal>, StoreError>> + Send;

    /// Resolve a signal: `Confirmed` or `Dismissed` (`Proposed` is not a
    /// resolution — `Invalid`), stamping who judged it. Re-resolving
    /// flips the verdict and the stamp; there is no path back to
    /// `proposed`.
    fn signal_resolve(
        &self,
        id: SignalId,
        status: SignalStatus,
        by: Author,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
