//! The session — a conversation container that decisions cite as evidence:
//! an agent transcript, a Slack thread, a PR discussion, an incident
//! channel. Messages stream into it append-only; decisions anchor to
//! specific messages (see [`crate::message`] and the evidence operations
//! on [`crate::decision::Decisions`]).

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{ProjectId, SessionId};
use crate::{Pagination, StoreError};

/// Where a conversation happened — the four source shapes the product
/// renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// An agent session transcript (Claude Code, claude.ai, …).
    Transcript,
    Slack,
    Pr,
    Incident,
}

/// A session, as stored and served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub kind: SessionKind,
    /// The source system's reference — the natural key's second half
    /// (a Claude session id, a Slack thread URL, a PR reference).
    pub external: String,
    pub title: String,
    /// When converge learned of it (server-assigned).
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

/// A session to ensure (`unique (kind, external)` is the identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSession {
    pub project_id: ProjectId,
    pub kind: SessionKind,
    pub external: String,
    pub title: String,
}

/// Narrowing for session lists; fields compose (AND).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFilter {
    pub project: Option<ProjectId>,
    pub kind: Option<SessionKind>,
}

/// Storage operations on sessions.
pub trait Sessions {
    /// Create-or-refresh by natural key `(kind, external)` — deterministic
    /// and race-safe (importers and live agents race on the same
    /// conversation). The **title refreshes** on every ensure (titles
    /// evolve); the **project binding stays** as first created — evidence
    /// doesn't silently re-home.
    fn session_ensure(
        &self,
        new: NewSession,
    ) -> impl Future<Output = Result<SessionId, StoreError>> + Send;

    fn session_get(
        &self,
        id: SessionId,
    ) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;

    /// Sessions, newest first.
    fn session_list(
        &self,
        filter: SessionFilter,
        page: Pagination<SessionId>,
    ) -> impl Future<Output = Result<Vec<Session>, StoreError>> + Send;
}
