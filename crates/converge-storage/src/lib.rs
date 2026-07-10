//! The storage seam for Converge — the **Repository traits** the product is
//! written against.
//!
//! The domain layer, HTTP/MCP server, CLI, and web sit *above* these traits;
//! storage sits *below* them. Keeping the seam here makes the backend
//! pluggable and the domain testable against a fake. The bundled backend is
//! `converge-storage-postgres` (PostgreSQL).
//!
//! Each resource module carries its types **and** its storage trait
//! ([`Groups`], [`Projects`], [`Decisions`], …); [`Storage`] bundles them
//! all. Consumers narrow to the trait they need (`fn f<S: Decisions>(…)`) or
//! take the whole surface (`S: Storage`); backends implement the per-resource
//! traits and get `Storage` from the blanket impl.
//!
//! Methods follow a `resource_operation` naming (`decision_add`,
//! `decision_get`, …): the bundle merges every trait into one method
//! namespace, so the resource prefix is what keeps names collision-free.
//! Edits are applied as an atomic batch of `*Edit` operations.
//! (Methods are spelled `-> impl Future + Send` so the returned futures are
//! `Send`.)

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod agent;
pub mod decision;
pub mod group;
pub mod ids;
pub mod message;
pub mod project;
pub mod session;
pub mod token;
pub mod user;

pub use agent::{Agent, AgentKind, Agents, NewAgent};
pub use decision::{
    Alternative, Author, Decision, DecisionEdit, DecisionFilter, DecisionStatus, Decisions, Edges,
    NewDecision, Related, Source,
};
pub use group::{Group, GroupEdit, GroupKind, Groups, NewGroup};
pub use ids::{AgentId, DecisionId, GroupId, MessageId, ProjectId, SessionId, TokenId, UserId};
pub use message::{Message, Messages, NewMessage};
pub use project::{NewProject, Project, ProjectEdit, ProjectFilter, Projects};
pub use session::{NewSession, Session, SessionFilter, SessionKind, Sessions};
pub use token::{Minted, NewToken, Token, Tokens};
pub use user::{AuthInfo, Identity, User, Users};

/// Cursor pagination for list reads, generic over the listed resource's id.
/// Lists are newest-first; `cursor` is the last id of the previous page and
/// only strictly older ids (ULID = time-ordered) are returned. Travels
/// separately from the per-resource filters: a filter says *what*, this
/// says *how much and from where*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pagination<Id> {
    pub limit: Option<u32>,
    pub cursor: Option<Id>,
}

// Manual: `derive(Default)` would demand `Id: Default`, and ids have no
// default by design.
impl<Id> Default for Pagination<Id> {
    fn default() -> Self {
        Self {
            limit: None,
            cursor: None,
        }
    }
}

/// One page of a list read — the response twin of [`Pagination`]. Pass
/// `next_cursor` back as the next request's `cursor`; `None` means the
/// list is exhausted (or the read was unpaginated). The cursor is an
/// opaque token — clients don't parse it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Wrap a list read: a full page (`len == limit`) points the cursor at
    /// its last item; a short or unlimited read ends the list.
    pub fn new<Id>(items: Vec<T>, page: &Pagination<Id>, id: impl Fn(&T) -> String) -> Self {
        let next_cursor = match page.limit {
            Some(limit) if limit > 0 && items.len() == limit as usize => items.last().map(id),
            _ => None,
        };
        Page { items, next_cursor }
    }
}

/// Backend-agnostic storage error. A backend maps its native failures into
/// these; callers distinguish only what they need to act on.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    /// Authentication required or refused. Produced by the HTTP layers
    /// (server gate, client) — storage backends never return it; it lives
    /// here because `StoreError` doubles as the wire error contract.
    #[error("unauthorized")]
    Unauthorized,
    /// Backend unreachable / transient — retryable, distinct from a logical failure.
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("backend error: {0}")]
    Backend(String),
}

/// The full storage surface — every resource trait, bundled. Implemented
/// automatically for any type that implements them all.
pub trait Storage:
    Groups + Projects + Users + Agents + Tokens + Decisions + Sessions + Messages + Clone + Send + Sync
{
}

impl<
    T: Groups
        + Projects
        + Users
        + Agents
        + Tokens
        + Decisions
        + Sessions
        + Messages
        + Clone
        + Send
        + Sync,
> Storage for T
{
}
