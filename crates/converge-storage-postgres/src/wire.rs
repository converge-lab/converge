//! Wire-format mapping: Postgres enum/row shapes ↔ domain types.
//!
//! The domain crate stays sqlx-free; the Postgres type names live only here.

use converge_storage::{Alternative, Decision, DecisionId, ProjectId, StoreError};
use time::OffsetDateTime;
use uuid::Uuid;

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
            id: DecisionId::from(ulid::Ulid::from(r.id)),
            project_id: ProjectId::from(ulid::Ulid::from(r.project_id)),
            status: r.status.into(),
            title: r.title,
            summary: r.summary,
            context: r.context,
            consequences: r.consequences,
            alternatives,
            // Authorship lands with the users/agents slice; until then nothing
            // is stored (decision_add rejects non-empty authors).
            authors: Vec::new(),
            captured_at: r.captured_at,
        })
    }
}
