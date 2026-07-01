//! PostgreSQL backend for Converge — implements [`converge_storage::Storage`].
//!
//! Ids are ULIDs stored as native `uuid` (same 128 bits, converted at this
//! boundary); the timestamp-first ULID layout keeps `order by id` = capture
//! order. Queries are compile-time checked (`sqlx::query!`) against the
//! committed `.sqlx/` cache — regenerate it with `cargo xtask prepare` after
//! changing any query.

mod wire;

use converge_storage::{
    Decision, DecisionEdit, DecisionFilter, DecisionId, DecisionStatus, Decisions, Edges, Group,
    GroupEdit, GroupId, Groups, NewDecision, NewGroup, NewProject, Project, ProjectEdit,
    ProjectFilter, ProjectId, Projects, Related, StoreError,
};
use sqlx::PgPool;
use uuid::Uuid;
use wire::DecisionStatus as PgStatus;
use wire::GroupKind as PgGroupKind;

/// Superseded is derived from inbound edges — storing it is a caller error.
const SUPERSEDED_IS_DERIVED: &str =
    "`superseded` is derived from supersedes edges; add an edge instead of setting the status";

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

impl Groups for PgStorage {
    async fn group_add(&self, new: NewGroup) -> Result<GroupId, StoreError> {
        let id = GroupId::new();
        let kind = PgGroupKind::from(new.kind);
        sqlx::query!(
            "insert into groups (id, name, description, kind) values ($1, $2, $3, $4)",
            Uuid::from(id.ulid()),
            new.name,
            new.description,
            kind as PgGroupKind,
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn group_get(&self, id: GroupId) -> Result<Option<Group>, StoreError> {
        Ok(sqlx::query_as!(
            wire::GroupRow,
            r#"select id, name, description, kind as "kind: _", created_at
               from groups where id = $1"#,
            Uuid::from(id.ulid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Group::from))
    }

    async fn group_list(&self) -> Result<Vec<Group>, StoreError> {
        Ok(sqlx::query_as!(
            wire::GroupRow,
            r#"select id, name, description, kind as "kind: _", created_at
               from groups order by id desc"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Group::from)
        .collect())
    }

    async fn group_edit(&self, id: GroupId, edits: Vec<GroupEdit>) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let held = sqlx::query!("select id from groups where id = $1 for update", uuid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        if held.is_none() {
            return Err(StoreError::NotFound);
        }
        for edit in edits {
            match edit {
                GroupEdit::SetName(name) => {
                    sqlx::query!("update groups set name = $2 where id = $1", uuid, name)
                        .execute(&mut *tx)
                        .await
                }
                GroupEdit::SetDescription(description) => {
                    sqlx::query!(
                        "update groups set description = $2 where id = $1",
                        uuid,
                        description,
                    )
                    .execute(&mut *tx)
                    .await
                }
            }
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)
    }
}

impl Projects for PgStorage {
    async fn project_add(&self, new: NewProject) -> Result<ProjectId, StoreError> {
        let id = ProjectId::new();
        sqlx::query!(
            "insert into projects (id, group_id, name, description) values ($1, $2, $3, $4)",
            Uuid::from(id.ulid()),
            Uuid::from(new.group_id.ulid()),
            new.name,
            new.description,
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn project_get(&self, id: ProjectId) -> Result<Option<Project>, StoreError> {
        Ok(sqlx::query_as!(
            wire::ProjectRow,
            "select id, group_id, name, description, created_at from projects where id = $1",
            Uuid::from(id.ulid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Project::from))
    }

    async fn project_list(&self, filter: ProjectFilter) -> Result<Vec<Project>, StoreError> {
        Ok(sqlx::query_as!(
            wire::ProjectRow,
            r#"select id, group_id, name, description, created_at
               from projects
               where ($1::uuid is null or group_id = $1)
               order by id desc
               limit $2"#,
            filter.group.map(|g| Uuid::from(g.ulid())),
            filter.limit.map(i64::from),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Project::from)
        .collect())
    }

    async fn project_edit(&self, id: ProjectId, edits: Vec<ProjectEdit>) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let held = sqlx::query!("select id from projects where id = $1 for update", uuid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        if held.is_none() {
            return Err(StoreError::NotFound);
        }
        for edit in edits {
            match edit {
                ProjectEdit::SetName(name) => {
                    sqlx::query!("update projects set name = $2 where id = $1", uuid, name)
                        .execute(&mut *tx)
                        .await
                }
                ProjectEdit::SetDescription(description) => {
                    sqlx::query!(
                        "update projects set description = $2 where id = $1",
                        uuid,
                        description,
                    )
                    .execute(&mut *tx)
                    .await
                }
            }
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)
    }
}

impl Decisions for PgStorage {
    async fn decision_add(&self, new: NewDecision) -> Result<DecisionId, StoreError> {
        if !new.authors.is_empty() {
            return Err(StoreError::Invalid(
                "authorship is not implemented yet; authors must be empty".into(),
            ));
        }
        if new.status == DecisionStatus::Superseded {
            return Err(StoreError::Invalid(SUPERSEDED_IS_DERIVED.into()));
        }
        let alternatives = serde_json::to_value(&new.alternatives)
            .map_err(|e| StoreError::Invalid(format!("alternatives: {e}")))?;
        let id = DecisionId::new();
        let status = PgStatus::from(new.status);
        let mut tx = self.pool.begin().await.map_err(db_err)?;
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
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        if !new.supersedes.is_empty() {
            let targets: Vec<Uuid> =
                new.supersedes.iter().map(|d| Uuid::from(d.ulid())).collect();
            sqlx::query!(
                r#"insert into decision_supersedes (decision_id, supersedes_id)
                   select $1, unnest($2::uuid[])
                   on conflict do nothing"#,
                Uuid::from(id.ulid()),
                &targets[..],
            )
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(id)
    }

    async fn decision_get(&self, id: DecisionId) -> Result<Option<Decision>, StoreError> {
        sqlx::query_as!(
            wire::DecisionRow,
            r#"select id, project_id,
                      case when exists (select 1 from decision_supersedes s
                                        where s.supersedes_id = d.id)
                           then 'superseded'::decision_status
                           else d.status
                      end as "status!: _",
                      title, summary, context, consequences, alternatives, captured_at
               from decisions d
               where d.id = $1"#,
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
        // `limit null` means no limit. The status filter matches the *derived*
        // status, hence the inner select. ULID ids sort by time — newest first.
        sqlx::query_as!(
            wire::DecisionRow,
            r#"select id as "id!", project_id as "project_id!", status as "status!: _",
                      title as "title!", summary as "summary!", context, consequences,
                      alternatives as "alternatives!", captured_at as "captured_at!"
               from (
                   select d.id, d.project_id, p.group_id,
                          case when exists (select 1 from decision_supersedes s
                                            where s.supersedes_id = d.id)
                               then 'superseded'::decision_status
                               else d.status
                          end as status,
                          d.title, d.summary, d.context, d.consequences,
                          d.alternatives, d.captured_at
                   from decisions d
                   join projects p on p.id = d.project_id
               ) d
               where ($1::uuid is null or d.project_id = $1)
                 and ($2::uuid is null or d.group_id = $2)
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

    async fn decision_edges(&self, id: DecisionId) -> Result<Option<Edges>, StoreError> {
        let uuid = Uuid::from(id.ulid());
        let exists = sqlx::query!("select id from decisions where id = $1", uuid)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        if exists.is_none() {
            return Ok(None);
        }
        let supersedes = sqlx::query_scalar!(
            "select supersedes_id from decision_supersedes where decision_id = $1
             order by supersedes_id",
            uuid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(wire::id)
        .collect();
        let superseded_by = sqlx::query_scalar!(
            "select decision_id from decision_supersedes where supersedes_id = $1
             order by decision_id",
            uuid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(wire::id)
        .collect();
        let related_to = sqlx::query!(
            "select ref_id, why from decision_related where decision_id = $1 order by ref_id",
            uuid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|r| Related { id: wire::id(r.ref_id), why: r.why })
        .collect();
        let related_by = sqlx::query!(
            "select decision_id, why from decision_related where ref_id = $1
             order by decision_id",
            uuid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|r| Related { id: wire::id(r.decision_id), why: r.why })
        .collect();
        Ok(Some(Edges { supersedes, superseded_by, related_to, related_by }))
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
            if s == DecisionStatus::Superseded {
                return Err(StoreError::Invalid(SUPERSEDED_IS_DERIVED.into()));
            }
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
        DecisionEdit::AddSupersedes(target) => {
            let target = no_self_loop(id, target, "supersede")?;
            sqlx::query!(
                "insert into decision_supersedes (decision_id, supersedes_id)
                 values ($1, $2) on conflict do nothing",
                id,
                target,
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::RemoveSupersedes(target) => {
            sqlx::query!(
                "delete from decision_supersedes where decision_id = $1 and supersedes_id = $2",
                id,
                Uuid::from(target.ulid()),
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::AddRelated { to, why } => {
            let to = no_self_loop(id, to, "cross-reference")?;
            sqlx::query!(
                "insert into decision_related (decision_id, ref_id, why)
                 values ($1, $2, $3)
                 on conflict (decision_id, ref_id) do update set why = excluded.why",
                id,
                to,
                why,
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::RemoveRelated(target) => {
            sqlx::query!(
                "delete from decision_related where decision_id = $1 and ref_id = $2",
                id,
                Uuid::from(target.ulid()),
            )
            .execute(&mut **tx)
            .await
        }
    }
    .map_err(db_err)?;
    Ok(())
}

/// Edge targets must be another decision (the schema checks this too).
fn no_self_loop(id: Uuid, target: DecisionId, verb: &str) -> Result<Uuid, StoreError> {
    let target = Uuid::from(target.ulid());
    if target == id {
        return Err(StoreError::Invalid(format!(
            "a decision cannot {verb} itself"
        )));
    }
    Ok(target)
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
