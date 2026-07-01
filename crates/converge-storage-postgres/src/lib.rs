//! PostgreSQL backend for Converge — implements [`converge_storage::Storage`].
//!
//! Ids are ULIDs stored as native `uuid` (same 128 bits, converted at this
//! boundary); the timestamp-first ULID layout keeps `order by id` = capture
//! order. Queries are compile-time checked (`sqlx::query!`) against the
//! committed `.sqlx/` cache — regenerate it with `cargo xtask prepare` after
//! changing any query.

mod wire;

use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, NewDecision, StoreError, Storage,
};
use sqlx::PgPool;
use uuid::Uuid;
use wire::DecisionStatus as PgStatus;

/// The embedded schema migrations (`./migrations`).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// PostgreSQL-backed storage. Cheap to clone (shares the connection pool).
#[derive(Clone)]
pub struct PgStorage {
    pool: PgPool,
}

impl PgStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connect to `url` — eagerly, so a bad URL fails here rather than on
    /// first use.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPool::connect(url).await.map_err(db_err)?;
        Ok(Self::new(pool))
    }

    /// Apply any pending schema migrations.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    /// The underlying pool, for embedding and tests.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl Storage for PgStorage {
    async fn decision_add(&self, new: NewDecision) -> Result<DecisionId, StoreError> {
        if !new.authors.is_empty() {
            return Err(StoreError::Invalid(
                "authorship is not implemented yet; authors must be empty".into(),
            ));
        }
        let alternatives = serde_json::to_value(&new.alternatives)
            .map_err(|e| StoreError::Invalid(format!("alternatives: {e}")))?;
        let id = DecisionId::new();
        let status = PgStatus::from(new.status);
        sqlx::query!(
            r#"insert into decisions
                   (id, project_id, status, title, summary, context, consequences, alternatives)
               values ($1, $2, $3, $4, $5, $6, $7, $8)"#,
            Uuid::from(id.ulid()),
            Uuid::from(new.project_id.ulid()),
            status as PgStatus,
            new.title,
            new.summary,
            new.context,
            new.consequences,
            alternatives,
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn decision_get(&self, id: DecisionId) -> Result<Option<Decision>, StoreError> {
        sqlx::query_as!(
            wire::DecisionRow,
            r#"select id, project_id, status as "status: _", title, summary,
                      context, consequences, alternatives, captured_at
               from decisions
               where id = $1"#,
            Uuid::from(id.ulid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Decision::try_from)
        .transpose()
    }

    async fn decision_list(&self, filter: DecisionFilter) -> Result<Vec<Decision>, StoreError> {
        let status = filter.status.map(PgStatus::from);
        // Static SQL (compile-checked): absent filters collapse to `$n is null`;
        // `limit null` means no limit. ULID ids sort by time, so newest first.
        sqlx::query_as!(
            wire::DecisionRow,
            r#"select d.id, d.project_id, d.status as "status: _", d.title, d.summary,
                      d.context, d.consequences, d.alternatives, d.captured_at
               from decisions d
               join projects p on p.id = d.project_id
               where ($1::uuid is null or d.project_id = $1)
                 and ($2::uuid is null or p.group_id = $2)
                 and ($3::decision_status is null or d.status = $3)
               order by d.id desc
               limit $4"#,
            filter.project.map(|p| Uuid::from(p.ulid())),
            filter.group.map(|g| Uuid::from(g.ulid())),
            status as Option<PgStatus>,
            filter.limit.map(i64::from),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Decision::try_from)
        .collect()
    }

    async fn decision_edit(
        &self,
        id: DecisionId,
        edits: Vec<DecisionEdit>,
    ) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // Lock the row for the batch; a missing decision is NotFound.
        let held = sqlx::query!("select id from decisions where id = $1 for update", uuid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        if held.is_none() {
            return Err(StoreError::NotFound);
        }
        for edit in edits {
            apply(&mut tx, uuid, edit).await?;
        }
        tx.commit().await.map_err(db_err)
    }
}

/// Apply one [`DecisionEdit`] to the locked row, inside the caller's
/// transaction — one static, compile-checked statement per variant.
async fn apply(
    tx: &mut sqlx::PgTransaction<'_>,
    id: Uuid,
    edit: DecisionEdit,
) -> Result<(), StoreError> {
    match edit {
        DecisionEdit::SetStatus(s) => {
            let s = PgStatus::from(s);
            sqlx::query!(
                "update decisions set status = $2 where id = $1",
                id,
                s as PgStatus,
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::SetTitle(title) => {
            sqlx::query!("update decisions set title = $2 where id = $1", id, title)
                .execute(&mut **tx)
                .await
        }
        DecisionEdit::SetSummary(summary) => {
            sqlx::query!("update decisions set summary = $2 where id = $1", id, summary)
                .execute(&mut **tx)
                .await
        }
        DecisionEdit::SetContext(context) => {
            sqlx::query!("update decisions set context = $2 where id = $1", id, context)
                .execute(&mut **tx)
                .await
        }
        DecisionEdit::SetConsequences(consequences) => {
            sqlx::query!(
                "update decisions set consequences = $2 where id = $1",
                id,
                consequences,
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::SetAlternatives(alternatives) => {
            let alternatives = serde_json::to_value(&alternatives)
                .map_err(|e| StoreError::Invalid(format!("alternatives: {e}")))?;
            sqlx::query!(
                "update decisions set alternatives = $2 where id = $1",
                id,
                alternatives,
            )
            .execute(&mut **tx)
            .await
        }
    }
    .map_err(db_err)?;
    Ok(())
}

/// Map sqlx failures onto the backend-agnostic [`StoreError`].
fn db_err(e: sqlx::Error) -> StoreError {
    match &e {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23503") => {
            // Foreign-key violation: the caller referenced a record that
            // doesn't exist (e.g. an unknown project).
            StoreError::Invalid(format!("missing referenced record: {d}"))
        }
        sqlx::Error::Database(d) if d.code().as_deref() == Some("23505") => {
            StoreError::Conflict(d.to_string())
        }
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
            StoreError::Unavailable(e.to_string())
        }
        _ => StoreError::Backend(e.to_string()),
    }
}
