//! PostgreSQL backend for Converge — implements [`converge_storage::Storage`].
//!
//! Skeleton: the `sqlx::PgPool` and the real queries arrive with the first
//! slice; the methods are stubbed so the workspace builds.

use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, NewDecision, StoreError, Storage,
};

/// PostgreSQL-backed storage. Cheap to clone (shares the connection pool).
#[derive(Clone)]
pub struct PgStorage {
    // pool: sqlx::PgPool,   // added with the first query slice
}

impl Storage for PgStorage {
    async fn decision_add(&self, _new: NewDecision) -> Result<DecisionId, StoreError> {
        todo!()
    }

    async fn decision_get(&self, _id: DecisionId) -> Result<Option<Decision>, StoreError> {
        todo!()
    }

    async fn decision_list(&self, _filter: DecisionFilter) -> Result<Vec<Decision>, StoreError> {
        todo!()
    }

    async fn decision_edit(
        &self,
        _id: DecisionId,
        _edits: Vec<DecisionEdit>,
    ) -> Result<(), StoreError> {
        todo!()
    }
}
