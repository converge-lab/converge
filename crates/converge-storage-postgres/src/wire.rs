//! Wire-format mapping: Postgres enum/row shapes ↔ domain types.
//!
//! The domain crate stays sqlx-free; the Postgres type names live only here.

use converge_storage::{
    Agent, Alternative, Author, Decision, Group, Message, Project, ProjectId, Session, Signal,
    StoreError, Token, User,
};
use time::OffsetDateTime;
use ulid::Ulid;
use uuid::Uuid;

/// Convert a stored `uuid` back into one of the domain id newtypes.
pub(crate) fn id<T: From<Ulid>>(u: Uuid) -> T {
    T::from(Ulid::from(u))
}

/// The `decision_status` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "decision_status", rename_all = "lowercase")]
pub(crate) enum DecisionStatus {
    Accepted,
    Draft,
    Proposed,
    Superseded,
    Rejected,
}

impl From<converge_storage::DecisionStatus> for DecisionStatus {
    fn from(s: converge_storage::DecisionStatus) -> Self {
        use converge_storage::DecisionStatus as D;
        match s {
            D::Accepted => Self::Accepted,
            D::Draft => Self::Draft,
            D::Proposed => Self::Proposed,
            D::Superseded => Self::Superseded,
            D::Rejected => Self::Rejected,
        }
    }
}

impl From<DecisionStatus> for converge_storage::DecisionStatus {
    fn from(s: DecisionStatus) -> Self {
        use DecisionStatus as P;
        match s {
            P::Accepted => Self::Accepted,
            P::Draft => Self::Draft,
            P::Proposed => Self::Proposed,
            P::Superseded => Self::Superseded,
            P::Rejected => Self::Rejected,
        }
    }
}

/// The `group_kind` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "group_kind", rename_all = "lowercase")]
pub(crate) enum GroupKind {
    Shared,
    Personal,
}

impl From<converge_storage::GroupKind> for GroupKind {
    fn from(k: converge_storage::GroupKind) -> Self {
        use converge_storage::GroupKind as D;
        match k {
            D::Shared => Self::Shared,
            D::Personal => Self::Personal,
        }
    }
}

impl From<GroupKind> for converge_storage::GroupKind {
    fn from(k: GroupKind) -> Self {
        use GroupKind as P;
        match k {
            P::Shared => Self::Shared,
            P::Personal => Self::Personal,
        }
    }
}

/// The `agent_kind` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "agent_kind", rename_all = "lowercase")]
pub(crate) enum AgentKind {
    Model,
    Tool,
}

impl From<converge_storage::AgentKind> for AgentKind {
    fn from(k: converge_storage::AgentKind) -> Self {
        use converge_storage::AgentKind as D;
        match k {
            D::Model => Self::Model,
            D::Tool => Self::Tool,
        }
    }
}

impl From<AgentKind> for converge_storage::AgentKind {
    fn from(k: AgentKind) -> Self {
        use AgentKind as P;
        match k {
            P::Model => Self::Model,
            P::Tool => Self::Tool,
        }
    }
}

/// One `users` row, as fetched.
pub(crate) struct UserRow {
    pub id: Uuid,
    pub provider: String,
    pub subject: String,
    pub handle: String,
    pub name: String,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        User {
            id: id(r.id),
            provider: r.provider,
            subject: r.subject,
            handle: r.handle,
            name: r.name,
        }
    }
}

/// One `tokens` row, as listed (the hash never leaves the backend).
pub(crate) struct TokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub label: String,
    pub created_at: OffsetDateTime,
}

impl From<TokenRow> for Token {
    fn from(r: TokenRow) -> Self {
        Token {
            id: id(r.id),
            user_id: id(r.user_id),
            label: r.label,
            created_at: r.created_at,
        }
    }
}

/// The `session_kind` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "session_kind", rename_all = "lowercase")]
pub(crate) enum SessionKind {
    Transcript,
    Slack,
    Pr,
    Incident,
}

impl From<converge_storage::SessionKind> for SessionKind {
    fn from(k: converge_storage::SessionKind) -> Self {
        use converge_storage::SessionKind as D;
        match k {
            D::Transcript => Self::Transcript,
            D::Slack => Self::Slack,
            D::Pr => Self::Pr,
            D::Incident => Self::Incident,
        }
    }
}

impl From<SessionKind> for converge_storage::SessionKind {
    fn from(k: SessionKind) -> Self {
        use SessionKind as P;
        match k {
            P::Transcript => Self::Transcript,
            P::Slack => Self::Slack,
            P::Pr => Self::Pr,
            P::Incident => Self::Incident,
        }
    }
}

/// One `sessions` row, as fetched.
pub(crate) struct SessionRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: SessionKind,
    pub external: String,
    pub title: String,
    pub captured_at: OffsetDateTime,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Self {
        Session {
            id: id(r.id),
            project_id: id(r.project_id),
            kind: r.kind.into(),
            external: r.external,
            title: r.title,
            captured_at: r.captured_at,
        }
    }
}

/// One `messages` row, as fetched.
pub(crate) struct MessageRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub seq: i32,
    pub speaker: String,
    pub body: String,
    pub sent_at: Option<OffsetDateTime>,
    pub captured_at: OffsetDateTime,
}

impl From<MessageRow> for Message {
    fn from(r: MessageRow) -> Self {
        Message {
            id: id(r.id),
            session_id: id(r.session_id),
            seq: r.seq,
            speaker: r.speaker,
            body: r.body,
            sent_at: r.sent_at,
            captured_at: r.captured_at,
        }
    }
}

/// One `agents` row, as fetched.
pub(crate) struct AgentRow {
    pub id: Uuid,
    pub kind: AgentKind,
    pub name: String,
}

impl From<AgentRow> for Agent {
    fn from(r: AgentRow) -> Self {
        Agent {
            id: id(r.id),
            kind: r.kind.into(),
            name: r.name,
        }
    }
}

/// One `decision_author` `(user_id?, agent_id?)` pair back into the
/// three-state [`Author`]. Both-null is unrepresentable in the domain and
/// checked out by the schema — hitting it means a corrupt row.
pub(crate) fn author(user: Option<Uuid>, agent: Option<Uuid>) -> Result<Author, StoreError> {
    match (user, agent) {
        (Some(u), None) => Ok(Author::User(id(u))),
        (None, Some(a)) => Ok(Author::Agent(id(a))),
        (Some(u), Some(a)) => Ok(Author::UserViaAgent {
            user: id(u),
            agent: id(a),
        }),
        (None, None) => Err(StoreError::Backend(
            "decision_author row with neither user nor agent".into(),
        )),
    }
}

/// An [`Author`] split into the `(user_id?, agent_id?)` column pair.
pub(crate) fn split(author: &Author) -> (Option<Uuid>, Option<Uuid>) {
    match author {
        Author::User(u) => (Some(Uuid::from(u.ulid())), None),
        Author::Agent(a) => (None, Some(Uuid::from(a.ulid()))),
        Author::UserViaAgent { user, agent } => (
            Some(Uuid::from(user.ulid())),
            Some(Uuid::from(agent.ulid())),
        ),
    }
}

/// One `groups` row, as fetched.
pub(crate) struct GroupRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub kind: GroupKind,
    pub created_at: OffsetDateTime,
}

impl From<GroupRow> for Group {
    fn from(r: GroupRow) -> Self {
        Group {
            id: id(r.id),
            name: r.name,
            description: r.description,
            kind: r.kind.into(),
            created_at: r.created_at,
        }
    }
}

/// One `projects` row, as fetched.
pub(crate) struct ProjectRow {
    pub id: Uuid,
    pub group_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: OffsetDateTime,
}

impl From<ProjectRow> for Project {
    fn from(r: ProjectRow) -> Self {
        Project {
            id: id(r.id),
            group_id: id(r.group_id),
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        }
    }
}

/// The `signal_tier` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "signal_tier", rename_all = "lowercase")]
pub(crate) enum Tier {
    Watch,
    Coordinate,
    Conflict,
}

impl From<converge_storage::Tier> for Tier {
    fn from(t: converge_storage::Tier) -> Self {
        use converge_storage::Tier as D;
        match t {
            D::Watch => Self::Watch,
            D::Coordinate => Self::Coordinate,
            D::Conflict => Self::Conflict,
        }
    }
}

impl From<Tier> for converge_storage::Tier {
    fn from(t: Tier) -> Self {
        use Tier as P;
        match t {
            P::Watch => Self::Watch,
            P::Coordinate => Self::Coordinate,
            P::Conflict => Self::Conflict,
        }
    }
}

/// The `signal_status` Postgres enum.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[sqlx(type_name = "signal_status", rename_all = "lowercase")]
pub(crate) enum SignalStatus {
    Proposed,
    Confirmed,
    Dismissed,
}

impl From<converge_storage::SignalStatus> for SignalStatus {
    fn from(s: converge_storage::SignalStatus) -> Self {
        use converge_storage::SignalStatus as D;
        match s {
            D::Proposed => Self::Proposed,
            D::Confirmed => Self::Confirmed,
            D::Dismissed => Self::Dismissed,
        }
    }
}

impl From<SignalStatus> for converge_storage::SignalStatus {
    fn from(s: SignalStatus) -> Self {
        use SignalStatus as P;
        match s {
            P::Proposed => Self::Proposed,
            P::Confirmed => Self::Confirmed,
            P::Dismissed => Self::Dismissed,
        }
    }
}

/// One `signals` row, as fetched (the target set is attached by the
/// caller from `signal_targets`).
pub(crate) struct SignalRow {
    pub id: Uuid,
    pub source: Uuid,
    pub kind: String,
    pub tier: Tier,
    pub status: SignalStatus,
    pub title: String,
    pub text: String,
    pub consequence: Option<String>,
    pub recommendation: Option<String>,
    pub produced_user: Option<Uuid>,
    pub produced_agent: Option<Uuid>,
    pub resolved_user: Option<Uuid>,
    pub resolved_agent: Option<Uuid>,
    pub captured_at: OffsetDateTime,
}

impl TryFrom<SignalRow> for Signal {
    type Error = StoreError;

    fn try_from(r: SignalRow) -> Result<Self, StoreError> {
        // Resolver: both-null means unresolved; anything else is an
        // author pair like decision_author.
        let resolved_by = match (r.resolved_user, r.resolved_agent) {
            (None, None) => None,
            (user, agent) => Some(author(user, agent)?),
        };
        Ok(Signal {
            id: id(r.id),
            source: id(r.source),
            targets: Vec::new(), // attached by the caller
            kind: r.kind,
            tier: r.tier.into(),
            status: r.status.into(),
            title: r.title,
            text: r.text,
            consequence: r.consequence,
            recommendation: r.recommendation,
            produced_by: author(r.produced_user, r.produced_agent)?,
            resolved_by,
            captured_at: r.captured_at,
        })
    }
}

/// One `decisions` row, as fetched.
pub(crate) struct DecisionRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub status: DecisionStatus,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    pub alternatives: serde_json::Value,
    pub captured_at: OffsetDateTime,
}

impl TryFrom<DecisionRow> for Decision {
    type Error = StoreError;

    fn try_from(r: DecisionRow) -> Result<Self, StoreError> {
        let alternatives: Vec<Alternative> = serde_json::from_value(r.alternatives)
            .map_err(|e| StoreError::Backend(format!("corrupt alternatives json: {e}")))?;
        Ok(Decision {
            id: id(r.id),
            project_id: id::<ProjectId>(r.project_id),
            status: r.status.into(),
            title: r.title,
            summary: r.summary,
            context: r.context,
            consequences: r.consequences,
            alternatives,
            // Attached by the caller — separate decision_author /
            // evidence reads.
            authors: Vec::new(),
            evidence: Vec::new(),
            captured_at: r.captured_at,
        })
    }
}
