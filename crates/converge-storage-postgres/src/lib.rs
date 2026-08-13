//! PostgreSQL backend for Converge — implements [`converge_storage::Storage`].
//!
//! Ids are ULIDs stored as native `uuid` (same 128 bits, converted at this
//! boundary); the timestamp-first ULID layout keeps `order by id` ≈ capture
//! order at millisecond resolution — same-millisecond ids order by their
//! random tails. That is fine everywhere it can happen: `order by id` stays
//! a total order (cursors never skip or repeat), conversation order rides
//! the explicit `seq`, and concurrent writes have no meaningful creation
//! order to preserve. If a future feature needs strict insertion order,
//! the ladder is a process-wide monotonic ULID generator, then a
//! DB-assigned sequence. Queries are compile-time checked (`sqlx::query!`)
//! against the committed `.sqlx/` cache — regenerate it with
//! `cargo xtask prepare` after changing any query.

mod wire;

use std::collections::HashMap;

use converge_storage::{
    Agent, AgentId, Agents, Author, Decision, DecisionEdit, DecisionFilter, DecisionId,
    DecisionStatus, Decisions, DeviceClaim, DeviceGrant, Devices, Edges, Group, GroupEdit, GroupId,
    Groups, Identity, Member, Memberships, Message, MessageId, Messages, NewAgent, NewDecision,
    NewDeviceGrant, NewGroup, NewMessage, NewProject, NewSession, NewSignal, Pagination, Project,
    ProjectEdit, ProjectFilter, ProjectId, Projects, Related, Scope, Session, SessionFilter,
    SessionId, Sessions, Signal, SignalFilter, SignalId, SignalStatus, Signals, Source, StoreError,
    Token, TokenId, Tokens, User, UserId, Users,
};
use sqlx::PgPool;
use uuid::Uuid;
use wire::AgentKind as PgAgentKind;
use wire::DecisionStatus as PgStatus;
use wire::DeviceStatus as PgDeviceStatus;
use wire::GroupKind as PgGroupKind;
use wire::SessionKind as PgSessionKind;
use wire::SignalStatus as PgSignalStatus;
use wire::Tier as PgTier;

/// Superseded is derived from inbound edges — storing it is a caller error.
const SUPERSEDED_IS_DERIVED: &str =
    "`superseded` is derived from supersedes edges; add an edge instead of setting the status";

/// [`Scope`] as the nullable SQL parameter every scoped query binds:
/// `NULL` (System) short-circuits the `group_visible(…)` predicate.
fn viewer(scope: Scope) -> Option<Uuid> {
    scope.user().map(|u| Uuid::from(u.ulid()))
}

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

    /// Ensure the bootstrap personal workspace exists — a single well-known
    /// "My workspace" the deployment lands in, so the first-run (empty-group)
    /// experience always has a home and the "new project" path always has a
    /// target group. Idempotent: keyed on a fixed sentinel id (the nil UUID),
    /// so re-running on every boot is a no-op — an `ensure`, never a
    /// scan-then-create. Today's shared-trust deployment has exactly one;
    /// per-user personal workspaces arrive with the ownership model.
    /// Returns whether the row was created (for a one-time boot log).
    pub async fn ensure_default_workspace(&self) -> Result<bool, StoreError> {
        let result = sqlx::query!(
            r#"insert into groups (id, name, kind)
               values ($1, 'My workspace', 'personal'::group_kind)
               on conflict (id) do nothing"#,
            Uuid::nil(),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.rows_affected() > 0)
    }

    /// The underlying pool, for embedding and tests.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Evidence anchors for a set of decisions — one query, grouped.
    async fn evidence(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<MessageId>>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query!(
            "select decision_id, message_id from evidence
             where decision_id = any($1)
             order by message_id",
            ids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let mut evidence: HashMap<Uuid, Vec<MessageId>> = HashMap::new();
        for row in rows {
            evidence
                .entry(row.decision_id)
                .or_default()
                .push(wire::id(row.message_id));
        }
        Ok(evidence)
    }

    /// Authors for a set of decisions — one query, grouped by decision.
    /// Ordering is stable (arbitrary but deterministic).
    async fn authors(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<Author>>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query!(
            "select decision_id, user_id, agent_id from decision_author
             where decision_id = any($1)
             order by user_id nulls last, agent_id",
            ids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let mut authors: HashMap<Uuid, Vec<Author>> = HashMap::new();
        for row in rows {
            authors
                .entry(row.decision_id)
                .or_default()
                .push(wire::author(row.user_id, row.agent_id)?);
        }
        Ok(authors)
    }
}

impl Users for PgStorage {
    async fn user_login(&self, identity: Identity) -> Result<UserId, StoreError> {
        // `(provider, subject)` decides who; the mutable fields refresh on
        // every login so provider-side renames propagate. On conflict the
        // freshly minted id is discarded and `returning` yields the
        // existing row.
        let row = sqlx::query!(
            r#"insert into users (id, provider, subject, handle, name)
               values ($1, $2, $3, $4, $5)
               on conflict (provider, subject)
               do update set handle = excluded.handle, name = excluded.name
               returning id"#,
            Uuid::from(UserId::new().ulid()),
            identity.provider,
            identity.subject,
            identity.handle,
            identity.name,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(wire::id(row.id))
    }

    async fn user_get(&self, id: UserId) -> Result<Option<User>, StoreError> {
        Ok(sqlx::query_as!(
            wire::UserRow,
            "select id, provider, subject, handle, name from users where id = $1",
            Uuid::from(id.ulid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(User::from))
    }

    async fn user_lookup(&self, handle: &str) -> Result<Vec<User>, StoreError> {
        Ok(sqlx::query_as!(
            wire::UserRow,
            "select id, provider, subject, handle, name from users where handle = $1 order by id",
            handle,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(User::from)
        .collect())
    }

    async fn user_list(
        &self,
        scope: Scope,
        page: Pagination<UserId>,
    ) -> Result<Vec<User>, StoreError> {
        Ok(sqlx::query_as!(
            wire::UserRow,
            r#"select id, provider, subject, handle, name from users
               where ($1::uuid is null or id < $1)
                 and ($3::uuid is null or id = $3 or shares_group(id, $3))
               order by id desc
               limit $2"#,
            page.cursor.map(|c| Uuid::from(c.ulid())),
            page.limit.map(i64::from),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(User::from)
        .collect())
    }
}

impl Tokens for PgStorage {
    async fn token_add(
        &self,
        user: UserId,
        label: String,
        hash: String,
    ) -> Result<TokenId, StoreError> {
        let id = TokenId::new();
        sqlx::query!(
            "insert into tokens (id, user_id, hash, label) values ($1, $2, $3, $4)",
            Uuid::from(id.ulid()),
            Uuid::from(user.ulid()),
            hash,
            label,
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn token_user(&self, hash: &str) -> Result<Option<UserId>, StoreError> {
        Ok(
            sqlx::query!("select user_id from tokens where hash = $1", hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_err)?
                .map(|r| wire::id(r.user_id)),
        )
    }

    async fn token_revoke(&self, user: UserId, id: TokenId) -> Result<(), StoreError> {
        let result = sqlx::query!(
            "delete from tokens where id = $1 and user_id = $2",
            Uuid::from(id.ulid()),
            Uuid::from(user.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        // Owner-scoped: someone else's token reads as absent.
        match result.rows_affected() {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    async fn token_list(
        &self,
        user: UserId,
        page: Pagination<TokenId>,
    ) -> Result<Vec<Token>, StoreError> {
        Ok(sqlx::query_as!(
            wire::TokenRow,
            r#"select id, user_id, label, created_at from tokens
               where user_id = $1
                 and ($2::uuid is null or id < $2)
               order by id desc
               limit $3"#,
            Uuid::from(user.ulid()),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            page.limit.map(i64::from),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Token::from)
        .collect())
    }
}

impl Agents for PgStorage {
    async fn agent_ensure(&self, new: NewAgent) -> Result<AgentId, StoreError> {
        let kind = PgAgentKind::from(new.kind);
        let row = sqlx::query!(
            r#"insert into agents (id, kind, name) values ($1, $2, $3)
               on conflict (kind, name) do update set name = excluded.name
               returning id"#,
            Uuid::from(AgentId::new().ulid()),
            kind as PgAgentKind,
            new.name,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(wire::id(row.id))
    }

    async fn agent_get(&self, id: AgentId) -> Result<Option<Agent>, StoreError> {
        Ok(sqlx::query_as!(
            wire::AgentRow,
            r#"select id, kind as "kind: _", name from agents where id = $1"#,
            Uuid::from(id.ulid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Agent::from))
    }

    async fn agent_list(&self, page: Pagination<AgentId>) -> Result<Vec<Agent>, StoreError> {
        Ok(sqlx::query_as!(
            wire::AgentRow,
            r#"select id, kind as "kind: _", name from agents
               where ($1::uuid is null or id < $1)
               order by id desc
               limit $2"#,
            page.cursor.map(|c| Uuid::from(c.ulid())),
            page.limit.map(i64::from),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Agent::from)
        .collect())
    }
}

impl Groups for PgStorage {
    async fn group_add(&self, owner: UserId, new: NewGroup) -> Result<GroupId, StoreError> {
        let id = GroupId::new();
        let kind = PgGroupKind::from(new.kind);
        sqlx::query!(
            r#"insert into groups (id, name, description, kind, owner_id)
               values ($1, $2, $3, $4, $5)"#,
            Uuid::from(id.ulid()),
            new.name,
            new.description,
            kind as PgGroupKind,
            Uuid::from(owner.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    async fn group_get(&self, scope: Scope, id: GroupId) -> Result<Option<Group>, StoreError> {
        sqlx::query_as!(
            wire::GroupRow,
            r#"select id, name, description, kind as "kind: _", owner_id, created_at
               from groups
               where id = $1
                 and ($2::uuid is null or group_visible(id, $2))"#,
            Uuid::from(id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Group::try_from)
        .transpose()
    }

    async fn group_list(
        &self,
        scope: Scope,
        page: Pagination<GroupId>,
    ) -> Result<Vec<Group>, StoreError> {
        sqlx::query_as!(
            wire::GroupRow,
            r#"select id, name, description, kind as "kind: _", owner_id, created_at
               from groups
               where ($1::uuid is null or id < $1)
                 and ($3::uuid is null or group_visible(id, $3))
               order by id desc
               limit $2"#,
            page.cursor.map(|c| Uuid::from(c.ulid())),
            page.limit.map(i64::from),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Group::try_from)
        .collect()
    }

    async fn group_edit(
        &self,
        scope: Scope,
        id: GroupId,
        edits: Vec<GroupEdit>,
    ) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let held = sqlx::query!(
            r#"select owner_id from groups
               where id = $1
                 and ($2::uuid is null or group_visible(id, $2))
               for update"#,
            uuid,
            viewer(scope),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        let Some(held) = held else {
            return Err(StoreError::NotFound);
        };
        // Owner-only: a member sees the group but doesn't manage it.
        if let Scope::User(caller) = scope
            && held.owner_id != Some(Uuid::from(caller.ulid()))
        {
            return Err(StoreError::Invalid(
                "only the group owner can edit the group".into(),
            ));
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

    async fn group_delete(&self, scope: Scope, id: GroupId) -> Result<(), StoreError> {
        // The sentinel default workspace is infrastructure, not data —
        // boot would recreate it empty, which reads as data loss.
        if Uuid::from(id.ulid()) == Uuid::nil() {
            return Err(StoreError::Invalid(
                "the default workspace cannot be deleted".into(),
            ));
        }
        self.owner_gate(scope, id).await?;
        sqlx::query!("delete from groups where id = $1", Uuid::from(id.ulid()))
            .execute(&self.pool)
            .await
            .map_err(evidence_pinned)?;
        Ok(())
    }
}

impl PgStorage {
    /// The owner gate for membership management: the group must be
    /// visible (`NotFound` otherwise) and the caller its owner
    /// (`Invalid` otherwise); `System` passes. Returns the owner.
    async fn owner_gate(&self, scope: Scope, group: GroupId) -> Result<UserId, StoreError> {
        let row = sqlx::query!(
            r#"select owner_id from groups
               where id = $1 and ($2::uuid is null or group_visible(id, $2))"#,
            Uuid::from(group.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or(StoreError::NotFound)?;
        let owner = row
            .owner_id
            .ok_or_else(|| StoreError::Backend("unowned group".into()))?;
        if let Scope::User(caller) = scope
            && owner != Uuid::from(caller.ulid())
        {
            return Err(StoreError::Invalid(
                "only the group owner can manage membership".into(),
            ));
        }
        Ok(wire::id(owner))
    }

    /// Membership reads need the group visible, nothing more.
    async fn visible_gate(&self, scope: Scope, group: GroupId) -> Result<(), StoreError> {
        let seen = sqlx::query_scalar!(
            r#"select ($2::uuid is null or group_visible(id, $2)) as "seen!"
               from groups where id = $1"#,
            Uuid::from(group.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match seen {
            Some(true) => Ok(()),
            _ => Err(StoreError::NotFound),
        }
    }
}

impl Memberships for PgStorage {
    async fn member_add(
        &self,
        scope: Scope,
        group: GroupId,
        user: UserId,
    ) -> Result<(), StoreError> {
        let owner = self.owner_gate(scope, group).await?;
        if user == owner {
            return Err(StoreError::Invalid(
                "the owner is already a member — ownership contains membership".into(),
            ));
        }
        let invited_by = match scope {
            Scope::User(caller) => caller,
            Scope::System => owner,
        };
        sqlx::query!(
            r#"insert into memberships (group_id, user_id, invited_by)
               values ($1, $2, $3)
               on conflict do nothing"#,
            Uuid::from(group.ulid()),
            Uuid::from(user.ulid()),
            Uuid::from(invited_by.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn member_list(&self, scope: Scope, group: GroupId) -> Result<Vec<Member>, StoreError> {
        self.visible_gate(scope, group).await?;
        // Ownership contains membership: the owner heads the list (their
        // "since" is the group's own creation).
        Ok(sqlx::query!(
            r#"select user_id as "user_id!", handle as "handle!", name as "name!",
                      invited_by as "invited_by!", created_at as "created_at!",
                      owner as "owner!"
               from (
                   select g.owner_id as user_id, u.handle, u.name,
                          g.owner_id as invited_by, g.created_at, true as owner
                   from groups g join users u on u.id = g.owner_id
                   where g.id = $1
                   union all
                   select m.user_id, u.handle, u.name, m.invited_by,
                          m.created_at, false as owner
                   from memberships m
                   join users u on u.id = m.user_id
                   where m.group_id = $1
               ) roster
               order by owner desc, created_at, user_id"#,
            Uuid::from(group.ulid()),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|r| Member {
            user_id: wire::id(r.user_id),
            handle: r.handle,
            name: r.name,
            invited_by: wire::id(r.invited_by),
            since: r.created_at,
            owner: r.owner,
        })
        .collect())
    }

    async fn member_remove(
        &self,
        scope: Scope,
        group: GroupId,
        user: UserId,
    ) -> Result<(), StoreError> {
        // The owner removes anyone; a member removes themself (leaving).
        // The owner has no membership row, so removing them is NotFound
        // by construction.
        match scope {
            Scope::System => {}
            Scope::User(caller) if caller == user => self.visible_gate(scope, group).await?,
            Scope::User(_) => {
                self.owner_gate(scope, group).await?;
            }
        }
        let result = sqlx::query!(
            "delete from memberships where group_id = $1 and user_id = $2",
            Uuid::from(group.ulid()),
            Uuid::from(user.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        match result.rows_affected() {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    async fn adopt(&self, owner: UserId) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            "update groups set owner_id = $1 where owner_id is null",
            Uuid::from(owner.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(result.rows_affected())
    }
}

impl Projects for PgStorage {
    async fn project_add(&self, scope: Scope, new: NewProject) -> Result<ProjectId, StoreError> {
        self.visible_gate(scope, new.group_id).await?;
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

    async fn project_get(
        &self,
        scope: Scope,
        id: ProjectId,
    ) -> Result<Option<Project>, StoreError> {
        Ok(sqlx::query_as!(
            wire::ProjectRow,
            r#"select id, group_id, name, description, created_at
               from projects
               where id = $1
                 and ($2::uuid is null or group_visible(group_id, $2))"#,
            Uuid::from(id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Project::from))
    }

    async fn project_list(
        &self,
        scope: Scope,
        filter: ProjectFilter,
        page: Pagination<ProjectId>,
    ) -> Result<Vec<Project>, StoreError> {
        Ok(sqlx::query_as!(
            wire::ProjectRow,
            r#"select id, group_id, name, description, created_at
               from projects
               where ($1::uuid is null or group_id = $1)
                 and ($3::uuid is null or id < $3)
                 and ($4::uuid is null or group_visible(group_id, $4))
               order by id desc
               limit $2"#,
            filter.group.map(|g| Uuid::from(g.ulid())),
            page.limit.map(i64::from),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Project::from)
        .collect())
    }

    async fn project_edit(
        &self,
        scope: Scope,
        id: ProjectId,
        edits: Vec<ProjectEdit>,
    ) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let held = sqlx::query!(
            r#"select id from projects
               where id = $1
                 and ($2::uuid is null or group_visible(group_id, $2))
               for update"#,
            uuid,
            viewer(scope),
        )
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

    async fn project_delete(&self, scope: Scope, id: ProjectId) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let row = sqlx::query!(
            r#"select group_id from projects
               where id = $1 and ($2::uuid is null or group_visible(group_id, $2))"#,
            uuid,
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or(StoreError::NotFound)?;
        // Deleting is managing the owning group, not just seeing it.
        self.owner_gate(scope, wire::id(row.group_id)).await?;
        sqlx::query!("delete from projects where id = $1", uuid)
            .execute(&self.pool)
            .await
            .map_err(evidence_pinned)?;
        Ok(())
    }
}

impl Sessions for PgStorage {
    async fn session_ensure(&self, scope: Scope, new: NewSession) -> Result<SessionId, StoreError> {
        // `(kind, external)` decides identity; the title refreshes (titles
        // evolve as conversations grow) while the project binding stays as
        // first created — evidence doesn't silently re-home. On conflict
        // the freshly minted id is discarded — but only when the existing
        // session is visible: refreshing the title of another group's
        // session would be a cross-group write, so an invisible collision
        // is a plain Conflict instead.
        let project = self
            .project_get(scope, new.project_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        let kind = PgSessionKind::from(new.kind);
        let row = sqlx::query!(
            r#"insert into sessions (id, project_id, kind, external, title)
               values ($1, $2, $3, $4, $5)
               on conflict (kind, external) do update set title = excluded.title
               where ($6::uuid is null
                      or group_visible((select p.group_id from projects p
                                        where p.id = sessions.project_id), $6))
               returning id"#,
            Uuid::from(SessionId::new().ulid()),
            Uuid::from(project.id.ulid()),
            kind as PgSessionKind,
            new.external,
            new.title,
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match row {
            Some(row) => Ok(wire::id(row.id)),
            // The conflict arbiter matched but its WHERE refused: the
            // identity is claimed by a session the caller cannot see.
            None => Err(StoreError::Conflict(
                "this session identity is already recorded outside your groups".into(),
            )),
        }
    }

    async fn session_get(
        &self,
        scope: Scope,
        id: SessionId,
    ) -> Result<Option<Session>, StoreError> {
        Ok(sqlx::query_as!(
            wire::SessionRow,
            r#"select s.id, s.project_id, s.kind as "kind: _", s.external, s.title, s.captured_at
               from sessions s
               join projects p on p.id = s.project_id
               where s.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            Uuid::from(id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(Session::from))
    }

    async fn session_list(
        &self,
        scope: Scope,
        filter: SessionFilter,
        page: Pagination<SessionId>,
    ) -> Result<Vec<Session>, StoreError> {
        let kind = filter.kind.map(PgSessionKind::from);
        Ok(sqlx::query_as!(
            wire::SessionRow,
            r#"select s.id, s.project_id, s.kind as "kind: _", s.external, s.title, s.captured_at
               from sessions s
               join projects p on p.id = s.project_id
               where ($1::uuid is null or s.project_id = $1)
                 and ($2::session_kind is null or s.kind = $2)
                 and ($4::uuid is null or s.id < $4)
                 and ($5::uuid is null or group_visible(p.group_id, $5))
               order by s.id desc
               limit $3"#,
            filter.project.map(|p| Uuid::from(p.ulid())),
            kind as Option<PgSessionKind>,
            page.limit.map(i64::from),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Session::from)
        .collect())
    }
}

impl Messages for PgStorage {
    async fn message_add(
        &self,
        scope: Scope,
        session: SessionId,
        new: Vec<NewMessage>,
    ) -> Result<Vec<MessageId>, StoreError> {
        let session = Uuid::from(session.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // The session's row lock serializes appends: concurrent batches
        // can't interleave or collide on seq. Missing — or invisible —
        // session → NotFound.
        let held = sqlx::query!(
            r#"select s.id from sessions s
               join projects p on p.id = s.project_id
               where s.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))
               for update of s"#,
            session,
            viewer(scope),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        if held.is_none() {
            return Err(StoreError::NotFound);
        }
        let base = sqlx::query_scalar!(
            r#"select coalesce(max(seq) + 1, 0) as "base!" from messages where session_id = $1"#,
            session,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        // A loop of inserts inside the one transaction: obviously correct,
        // and evidence batches are conversation-sized. Bulk unnest can come
        // when an importer proves it matters.
        let mut ids = Vec::with_capacity(new.len());
        for (offset, message) in new.into_iter().enumerate() {
            let id = MessageId::new();
            sqlx::query!(
                r#"insert into messages (id, session_id, seq, speaker, body, sent_at)
                   values ($1, $2, $3, $4, $5, $6)"#,
                Uuid::from(id.ulid()),
                session,
                base + offset as i32,
                message.speaker,
                message.body,
                message.sent_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            ids.push(id);
        }
        tx.commit().await.map_err(db_err)?;
        Ok(ids)
    }

    async fn message_list(
        &self,
        scope: Scope,
        session: SessionId,
        page: Pagination<MessageId>,
    ) -> Result<Vec<Message>, StoreError> {
        // Conversation order — oldest first, the one forward-reading list;
        // the cursor returns rows strictly *after* it. An invisible
        // session reads as empty, same as an unknown one.
        Ok(sqlx::query_as!(
            wire::MessageRow,
            r#"select m.id, m.session_id, m.seq, m.speaker, m.body, m.sent_at, m.captured_at
               from messages m
               where m.session_id = $1
                 and ($2::uuid is null
                      or m.seq > (select seq from messages where id = $2 and session_id = $1))
                 and ($4::uuid is null
                      or group_visible((select p.group_id from sessions s
                                        join projects p on p.id = s.project_id
                                        where s.id = m.session_id), $4))
               order by m.seq
               limit $3"#,
            Uuid::from(session.ulid()),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            page.limit.map(i64::from),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Message::from)
        .collect())
    }
}

impl Decisions for PgStorage {
    async fn decision_add(&self, scope: Scope, new: NewDecision) -> Result<DecisionId, StoreError> {
        if new.status == DecisionStatus::Superseded {
            return Err(StoreError::Invalid(SUPERSEDED_IS_DERIVED.into()));
        }
        let alternatives = serde_json::to_value(&new.alternatives)
            .map_err(|e| StoreError::Invalid(format!("alternatives: {e}")))?;
        // The project must be visible to the writer, and every reference
        // (supersedes, evidence) must live in the SAME group — edges never
        // leave a group, so no read can ever surface an invisible endpoint.
        let group = sqlx::query_scalar!(
            r#"select p.group_id from projects p
               where p.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            Uuid::from(new.project_id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| StoreError::Invalid("missing referenced record: project".into()))?;
        if !new.supersedes.is_empty() {
            let mut targets: Vec<Uuid> = new
                .supersedes
                .iter()
                .map(|d| Uuid::from(d.ulid()))
                .collect();
            targets.sort();
            targets.dedup();
            let sound = sqlx::query_scalar!(
                r#"select count(*) as "n!" from decisions d
                   join projects p on p.id = d.project_id
                   where d.id = any($1) and p.group_id = $2"#,
                &targets[..],
                group,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
            if sound as usize != targets.len() {
                return Err(StoreError::Invalid(
                    "supersedes must reference decisions in the same group".into(),
                ));
            }
        }
        if !new.evidence.is_empty() {
            let mut anchors: Vec<Uuid> =
                new.evidence.iter().map(|m| Uuid::from(m.ulid())).collect();
            anchors.sort();
            anchors.dedup();
            let sound = sqlx::query_scalar!(
                r#"select count(*) as "n!" from messages m
                   join sessions s on s.id = m.session_id
                   join projects p on p.id = s.project_id
                   where m.id = any($1) and p.group_id = $2"#,
                &anchors[..],
                group,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
            if sound as usize != anchors.len() {
                return Err(StoreError::Invalid(
                    "evidence must reference messages in the same group".into(),
                ));
            }
        }
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
        if !new.authors.is_empty() {
            // Parallel (user?, agent?) arrays, one row per author; the
            // unique arbiter collapses duplicates within the batch too.
            let (users, agents): (Vec<_>, Vec<_>) = new.authors.iter().map(wire::split).unzip();
            sqlx::query!(
                r#"insert into decision_author (decision_id, user_id, agent_id)
                   select $1, a.user_id, a.agent_id
                   from unnest($2::uuid[], $3::uuid[]) as a(user_id, agent_id)
                   on conflict do nothing"#,
                Uuid::from(id.ulid()),
                &users as &[Option<Uuid>],
                &agents as &[Option<Uuid>],
            )
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        if !new.supersedes.is_empty() {
            let targets: Vec<Uuid> = new
                .supersedes
                .iter()
                .map(|d| Uuid::from(d.ulid()))
                .collect();
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
        if !new.evidence.is_empty() {
            let anchors: Vec<Uuid> = new.evidence.iter().map(|m| Uuid::from(m.ulid())).collect();
            sqlx::query!(
                r#"insert into evidence (decision_id, message_id)
                   select $1, unnest($2::uuid[])
                   on conflict do nothing"#,
                Uuid::from(id.ulid()),
                &anchors[..],
            )
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(id)
    }

    async fn decision_get(
        &self,
        scope: Scope,
        id: DecisionId,
    ) -> Result<Option<Decision>, StoreError> {
        let row = sqlx::query_as!(
            wire::DecisionRow,
            r#"select d.id, d.project_id,
                      case when exists (select 1 from decision_supersedes s
                                        where s.supersedes_id = d.id)
                           then 'superseded'::decision_status
                           else d.status
                      end as "status!: _",
                      d.title, d.summary, d.context, d.consequences, d.alternatives, d.captured_at
               from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            Uuid::from(id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        let uuid = row.id;
        let mut decision = Decision::try_from(row)?;
        decision.authors = self
            .authors(&[uuid])
            .await?
            .remove(&uuid)
            .unwrap_or_default();
        decision.evidence = self
            .evidence(&[uuid])
            .await?
            .remove(&uuid)
            .unwrap_or_default();
        Ok(Some(decision))
    }

    async fn decision_list(
        &self,
        scope: Scope,
        filter: DecisionFilter,
        page: Pagination<DecisionId>,
    ) -> Result<Vec<Decision>, StoreError> {
        let status = filter.status.map(PgStatus::from);
        // Static SQL (compile-checked): absent filters collapse to `$n is null`;
        // `limit null` means no limit. The status filter matches the *derived*
        // status, hence the inner select. ULID ids sort by time — newest first.
        let mut decisions = sqlx::query_as!(
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
                 and ($5::uuid is null or d.id < $5)
                 and ($6::uuid is null or group_visible(d.group_id, $6))
               order by d.id desc
               limit $4"#,
            filter.project.map(|p| Uuid::from(p.ulid())),
            filter.group.map(|g| Uuid::from(g.ulid())),
            status as Option<PgStatus>,
            page.limit.map(i64::from),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Decision::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let ids: Vec<Uuid> = decisions.iter().map(|d| Uuid::from(d.id.ulid())).collect();
        let mut authors = self.authors(&ids).await?;
        let mut evidence = self.evidence(&ids).await?;
        for decision in &mut decisions {
            let uuid = Uuid::from(decision.id.ulid());
            decision.authors = authors.remove(&uuid).unwrap_or_default();
            decision.evidence = evidence.remove(&uuid).unwrap_or_default();
        }
        Ok(decisions)
    }

    async fn decision_search(
        &self,
        scope: Scope,
        query: &str,
        filter: DecisionFilter,
        limit: Option<u32>,
    ) -> Result<Vec<Decision>, StoreError> {
        if query.trim().is_empty() {
            return Err(StoreError::Invalid("search needs a query".into()));
        }
        let status = filter.status.map(PgStatus::from);
        // websearch_to_tsquery never errors on user input; a query that is
        // all syntax ("-", "or") parses to the empty tsquery, which would
        // match nothing anyway — surface that as Invalid instead of an
        // empty page pretending to be a result.
        let mut decisions = sqlx::query_as!(
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
                          d.alternatives, d.captured_at, d.search
                   from decisions d
                   join projects p on p.id = d.project_id
               ) d
               where d.search @@ websearch_to_tsquery('english', $1)
                 and ($2::uuid is null or d.project_id = $2)
                 and ($3::uuid is null or d.group_id = $3)
                 and ($4::decision_status is null or d.status = $4)
                 and ($6::uuid is null or group_visible(d.group_id, $6))
               order by ts_rank_cd(d.search, websearch_to_tsquery('english', $1)) desc,
                        d.id desc
               limit $5"#,
            query,
            filter.project.map(|p| Uuid::from(p.ulid())),
            filter.group.map(|g| Uuid::from(g.ulid())),
            status as Option<PgStatus>,
            limit.map(i64::from),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Decision::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        if decisions.is_empty() {
            let empty = sqlx::query_scalar!(
                r#"select numnode(websearch_to_tsquery('english', $1)) = 0 as "empty!""#,
                query,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
            if empty {
                return Err(StoreError::Invalid(
                    "the query has no searchable terms".into(),
                ));
            }
        }
        let ids: Vec<Uuid> = decisions.iter().map(|d| Uuid::from(d.id.ulid())).collect();
        let mut authors = self.authors(&ids).await?;
        let mut evidence = self.evidence(&ids).await?;
        for decision in &mut decisions {
            let uuid = Uuid::from(decision.id.ulid());
            decision.authors = authors.remove(&uuid).unwrap_or_default();
            decision.evidence = evidence.remove(&uuid).unwrap_or_default();
        }
        Ok(decisions)
    }

    async fn decision_edit(
        &self,
        scope: Scope,
        id: DecisionId,
        edits: Vec<DecisionEdit>,
    ) -> Result<(), StoreError> {
        let uuid = Uuid::from(id.ulid());
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // Lock the row for the batch; a missing — or invisible — decision
        // is NotFound. The group rides along for edge-edit validation.
        let held = sqlx::query!(
            r#"select d.id, p.group_id from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))
               for update of d"#,
            uuid,
            viewer(scope),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        let Some(held) = held else {
            return Err(StoreError::NotFound);
        };
        for edit in edits {
            apply(&mut tx, uuid, held.group_id, edit).await?;
        }
        tx.commit().await.map_err(db_err)
    }

    async fn decision_sources(
        &self,
        scope: Scope,
        id: DecisionId,
    ) -> Result<Option<Vec<Source>>, StoreError> {
        /// How many messages of context to carry on each side of an anchor.
        const CONTEXT: i32 = 2;

        let uuid = Uuid::from(id.ulid());
        let exists = sqlx::query!(
            r#"select d.id from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            uuid,
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        if exists.is_none() {
            return Ok(None);
        }
        let anchors: Vec<Uuid> = sqlx::query_scalar!(
            "select message_id from evidence where decision_id = $1",
            uuid,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        if anchors.is_empty() {
            return Ok(Some(Vec::new()));
        }

        // The whole excerpt set in one pass: every message within CONTEXT
        // of any anchor of this decision, in (session, seq) order —
        // overlapping windows deduplicate for free.
        let windows = sqlx::query_as!(
            wire::MessageRow,
            r#"select m.id, m.session_id, m.seq, m.speaker, m.body, m.sent_at, m.captured_at
               from messages m
               where exists (
                   select 1
                   from evidence e
                   join messages a on a.id = e.message_id
                   where e.decision_id = $1
                     and a.session_id = m.session_id
                     and m.seq between a.seq - $2 and a.seq + $2
               )
               order by m.session_id, m.seq"#,
            uuid,
            CONTEXT,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;

        let session_ids: Vec<Uuid> = {
            let mut ids: Vec<Uuid> = windows.iter().map(|m| m.session_id).collect();
            ids.dedup();
            ids
        };
        let sessions: HashMap<Uuid, Session> = sqlx::query_as!(
            wire::SessionRow,
            r#"select id, project_id, kind as "kind: _", external, title, captured_at
               from sessions where id = any($1)"#,
            &session_ids[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|row| (row.id, Session::from(row)))
        .collect();

        // Group the ordered window rows into per-session sources, newest
        // session first (ULID order, matching every other list).
        let mut sources: Vec<Source> = Vec::new();
        for row in windows {
            let session_uuid = row.session_id;
            let message = Message::from(row);
            let current = match sources.last_mut() {
                Some(source) if Uuid::from(source.session.id.ulid()) == session_uuid => {
                    sources.last_mut().expect("just matched")
                }
                _ => {
                    let session = sessions
                        .get(&session_uuid)
                        .cloned()
                        .ok_or_else(|| StoreError::Backend("window row without session".into()))?;
                    sources.push(Source {
                        session,
                        messages: Vec::new(),
                        anchors: Vec::new(),
                    });
                    sources.last_mut().expect("just pushed")
                }
            };
            if anchors.contains(&Uuid::from(message.id.ulid())) {
                current.anchors.push(message.id);
            }
            current.messages.push(message);
        }
        sources.sort_by_key(|s| std::cmp::Reverse(s.session.id.ulid()));
        Ok(Some(sources))
    }

    async fn decision_edges(
        &self,
        scope: Scope,
        id: DecisionId,
    ) -> Result<Option<Edges>, StoreError> {
        let uuid = Uuid::from(id.ulid());
        // Edge endpoints are same-group by construction (validated at
        // every write), so gating the root decision covers them all.
        let exists = sqlx::query!(
            r#"select d.id from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            uuid,
            viewer(scope),
        )
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
        .map(|r| Related {
            id: wire::id(r.ref_id),
            why: r.why,
        })
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
        .map(|r| Related {
            id: wire::id(r.decision_id),
            why: r.why,
        })
        .collect();
        Ok(Some(Edges {
            supersedes,
            superseded_by,
            related_to,
            related_by,
        }))
    }
}

impl PgStorage {
    /// Target sets for a batch of signals — one query, grouped.
    async fn targets(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<DecisionId>>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query!(
            "select signal_id, target from signal_targets
             where signal_id = any($1)
             order by target",
            ids,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let mut targets: HashMap<Uuid, Vec<DecisionId>> = HashMap::new();
        for row in rows {
            targets
                .entry(row.signal_id)
                .or_default()
                .push(wire::id(row.target));
        }
        Ok(targets)
    }
}

impl Signals for PgStorage {
    async fn signal_add(&self, scope: Scope, new: NewSignal) -> Result<SignalId, StoreError> {
        let kind = new.kind.trim();
        if kind.is_empty() {
            return Err(StoreError::Invalid("kind must not be empty".into()));
        }
        let source = Uuid::from(new.source.ulid());
        let mut targets: Vec<Uuid> = new.targets.iter().map(|t| Uuid::from(t.ulid())).collect();
        targets.sort();
        targets.dedup();
        if targets.is_empty() {
            return Err(StoreError::Invalid(
                "at least one target is required".into(),
            ));
        }
        if targets.contains(&source) {
            return Err(StoreError::Invalid(
                "a signal cannot target its own source".into(),
            ));
        }
        // The source must be visible to the writer, and every target in
        // the source's group — signals never cross groups (the expert's
        // retrieval is group-scoped; this makes it structural).
        let group = sqlx::query_scalar!(
            r#"select p.group_id from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            source,
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| StoreError::Invalid("missing referenced record: source".into()))?;
        let sound = sqlx::query_scalar!(
            r#"select count(*) as "n!" from decisions d
               join projects p on p.id = d.project_id
               where d.id = any($1) and p.group_id = $2"#,
            &targets[..],
            group,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        if sound as usize != targets.len() {
            return Err(StoreError::Invalid(
                "targets must be decisions in the source's group".into(),
            ));
        }
        let (produced_user, produced_agent) = wire::split(&new.produced_by);
        let id = SignalId::new();
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // The (source, target, kind) uniqueness spans two tables, so it is
        // enforced here, not by a constraint: serialize concurrent adds on
        // the same (source, kind) with an advisory lock, then guard.
        sqlx::query!(
            "select pg_advisory_xact_lock(hashtextextended($1::text || $2, 0))",
            source.to_string(),
            kind,
        )
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        let raised = sqlx::query_scalar!(
            r#"select exists (
                   select 1 from signals s
                   join signal_targets t on t.signal_id = s.id
                   where s.source = $1 and s.kind = $2 and t.target = any($3)
               ) as "raised!""#,
            source,
            kind,
            &targets[..],
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if raised {
            return Err(StoreError::Conflict(format!(
                "a `{kind}` signal from this source already covers a requested target \
                 (possibly dismissed — dismissed observations are not re-raised)"
            )));
        }
        sqlx::query!(
            r#"insert into signals
                   (id, source, kind, tier, title, text, consequence, recommendation,
                    produced_user, produced_agent)
               values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
            Uuid::from(id.ulid()),
            source,
            kind,
            PgTier::from(new.tier) as PgTier,
            new.title,
            new.text,
            new.consequence,
            new.recommendation,
            produced_user,
            produced_agent,
        )
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        sqlx::query!(
            r#"insert into signal_targets (signal_id, target)
               select $1, unnest($2::uuid[])"#,
            Uuid::from(id.ulid()),
            &targets[..],
        )
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        Ok(id)
    }

    async fn signal_get(&self, scope: Scope, id: SignalId) -> Result<Option<Signal>, StoreError> {
        let row = sqlx::query_as!(
            wire::SignalRow,
            r#"select s.id, s.source, s.kind, s.tier as "tier: _", s.status as "status: _",
                      s.title, s.text, s.consequence, s.recommendation,
                      s.produced_user, s.produced_agent, s.resolved_user, s.resolved_agent,
                      s.captured_at
               from signals s
               join decisions d on d.id = s.source
               join projects p on p.id = d.project_id
               where s.id = $1
                 and ($2::uuid is null or group_visible(p.group_id, $2))"#,
            Uuid::from(id.ulid()),
            viewer(scope),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        let Some(row) = row else { return Ok(None) };
        let uuid = row.id;
        let mut signal = Signal::try_from(row)?;
        signal.targets = self
            .targets(&[uuid])
            .await?
            .remove(&uuid)
            .unwrap_or_default();
        Ok(Some(signal))
    }

    async fn signal_list(
        &self,
        scope: Scope,
        filter: SignalFilter,
        page: Pagination<SignalId>,
    ) -> Result<Vec<Signal>, StoreError> {
        let status = filter.status.map(PgSignalStatus::from);
        let tier = filter.tier.map(PgTier::from);
        // Project and decision filters match either end: the source
        // decision's project, or any target's. Visibility needs only the
        // source's group — targets are same-group by construction.
        let mut signals = sqlx::query_as!(
            wire::SignalRow,
            r#"select s.id, s.source, s.kind, s.tier as "tier: _", s.status as "status: _",
                      s.title, s.text, s.consequence, s.recommendation,
                      s.produced_user, s.produced_agent, s.resolved_user, s.resolved_agent,
                      s.captured_at
               from signals s
               where ($1::uuid is null
                      or exists (select 1 from decisions d
                                 where d.id = s.source and d.project_id = $1)
                      or exists (select 1 from signal_targets t
                                 join decisions d on d.id = t.target
                                 where t.signal_id = s.id and d.project_id = $1))
                 and ($2::uuid is null
                      or s.source = $2
                      or exists (select 1 from signal_targets t
                                 where t.signal_id = s.id and t.target = $2))
                 and ($3::signal_status is null or s.status = $3)
                 and ($4::signal_tier is null or s.tier = $4)
                 and ($6::uuid is null or s.id < $6)
                 and ($7::uuid is null
                      or group_visible((select p.group_id from decisions d
                                        join projects p on p.id = d.project_id
                                        where d.id = s.source), $7))
               order by s.id desc
               limit $5"#,
            filter.project.map(|p| Uuid::from(p.ulid())),
            filter.decision.map(|d| Uuid::from(d.ulid())),
            status as Option<PgSignalStatus>,
            tier as Option<PgTier>,
            page.limit.map(i64::from),
            page.cursor.map(|c| Uuid::from(c.ulid())),
            viewer(scope),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(Signal::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let ids: Vec<Uuid> = signals.iter().map(|s| Uuid::from(s.id.ulid())).collect();
        let mut targets = self.targets(&ids).await?;
        for signal in &mut signals {
            signal.targets = targets
                .remove(&Uuid::from(signal.id.ulid()))
                .unwrap_or_default();
        }
        Ok(signals)
    }

    async fn signal_resolve(
        &self,
        scope: Scope,
        id: SignalId,
        status: SignalStatus,
        by: Author,
    ) -> Result<(), StoreError> {
        if status == SignalStatus::Proposed {
            return Err(StoreError::Invalid(
                "`proposed` is not a resolution — confirm or dismiss".into(),
            ));
        }
        let (user, agent) = wire::split(&by);
        let result = sqlx::query!(
            r#"update signals
               set status = $2, resolved_user = $3, resolved_agent = $4
               where id = $1
                 and ($5::uuid is null
                      or group_visible((select p.group_id from decisions d
                                        join projects p on p.id = d.project_id
                                        where d.id = signals.source), $5))"#,
            Uuid::from(id.ulid()),
            PgSignalStatus::from(status) as PgSignalStatus,
            user,
            agent,
            viewer(scope),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}

impl Devices for PgStorage {
    async fn device_start(&self, new: NewDeviceGrant) -> Result<(), StoreError> {
        // Opportunistic sweep: expired grants are dead weight nobody will
        // claim once the poller gives up — clear them as new ones open.
        sqlx::query!("delete from device_grants where expires_at <= now()")
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        sqlx::query!(
            r#"insert into device_grants
                   (device_hash, client_hash, user_code, client_name, expires_at)
               values ($1, $2, $3, $4, $5)"#,
            new.device_hash,
            new.client_hash,
            new.user_code,
            new.client_name,
            new.expires_at,
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn device_get(&self, user_code: &str) -> Result<Option<DeviceGrant>, StoreError> {
        Ok(sqlx::query!(
            r#"select user_code, client_name, expires_at from device_grants
               where user_code = $1
                 and status = 'pending'::device_status
                 and expires_at > now()"#,
            user_code,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .map(|r| DeviceGrant {
            user_code: r.user_code,
            client_name: r.client_name,
            expires_at: r.expires_at,
        }))
    }

    async fn device_decide(
        &self,
        user_code: &str,
        user: UserId,
        approve: bool,
    ) -> Result<(), StoreError> {
        let status = match approve {
            true => PgDeviceStatus::Approved,
            false => PgDeviceStatus::Denied,
        };
        let result = sqlx::query!(
            r#"update device_grants
               set status = $2, user_id = $3
               where user_code = $1
                 and status = 'pending'::device_status
                 and expires_at > now()"#,
            user_code,
            status as PgDeviceStatus,
            Uuid::from(user.ulid()),
        )
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        match result.rows_affected() {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    async fn device_claim(
        &self,
        device_hash: &str,
        client_hash: &str,
    ) -> Result<DeviceClaim, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let row = sqlx::query!(
            r#"select client_hash, status as "status: PgDeviceStatus", user_id,
                      expires_at > now() as "live!"
               from device_grants where device_hash = $1 for update"#,
            device_hash,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        let Some(row) = row else {
            return Ok(DeviceClaim::Gone);
        };
        // Bound to the opener: a poll with the wrong client reads as Gone
        // and does not consume — the real client's poll is unaffected.
        if row.client_hash != client_hash {
            return Ok(DeviceClaim::Gone);
        }
        let claim = match row.status {
            PgDeviceStatus::Pending if row.live => return Ok(DeviceClaim::Pending),
            PgDeviceStatus::Pending => DeviceClaim::Gone,
            PgDeviceStatus::Approved => {
                let user = row
                    .user_id
                    .ok_or_else(|| StoreError::Backend("approved grant without a user".into()))?;
                DeviceClaim::Approved(wire::id(user))
            }
            PgDeviceStatus::Denied => DeviceClaim::Denied,
        };
        // Terminal — consume the row; a device code never yields twice.
        sqlx::query!(
            "delete from device_grants where device_hash = $1",
            device_hash,
        )
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        Ok(claim)
    }
}

/// A decision referenced by an edge edit must live in `group` — edges
/// never leave a group (see `Decisions::decision_add`).
async fn same_group(
    tx: &mut sqlx::PgTransaction<'_>,
    target: Uuid,
    group: Uuid,
    what: &str,
) -> Result<(), StoreError> {
    let sound = sqlx::query_scalar!(
        r#"select exists (
               select 1 from decisions d
               join projects p on p.id = d.project_id
               where d.id = $1 and p.group_id = $2
           ) as "sound!""#,
        target,
        group,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    if !sound {
        return Err(StoreError::Invalid(format!(
            "{what} must reference a decision in the same group"
        )));
    }
    Ok(())
}

/// Apply one [`DecisionEdit`] to the locked row, inside the caller's
/// transaction — one static, compile-checked statement per variant.
/// `group` is the decision's group, for edge validation.
async fn apply(
    tx: &mut sqlx::PgTransaction<'_>,
    id: Uuid,
    group: Uuid,
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
            sqlx::query!(
                "update decisions set summary = $2 where id = $1",
                id,
                summary
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::SetContext(context) => {
            sqlx::query!(
                "update decisions set context = $2 where id = $1",
                id,
                context
            )
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
            same_group(tx, target, group, "supersedes").await?;
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
            same_group(tx, to, group, "a cross-reference").await?;
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
        DecisionEdit::AddEvidence(message) => {
            let message = Uuid::from(message.ulid());
            let sound = sqlx::query_scalar!(
                r#"select exists (
                       select 1 from messages m
                       join sessions s on s.id = m.session_id
                       join projects p on p.id = s.project_id
                       where m.id = $1 and p.group_id = $2
                   ) as "sound!""#,
                message,
                group,
            )
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
            if !sound {
                return Err(StoreError::Invalid(
                    "evidence must reference a message in the same group".into(),
                ));
            }
            sqlx::query!(
                r#"insert into evidence (decision_id, message_id)
                   values ($1, $2)
                   on conflict do nothing"#,
                id,
                message,
            )
            .execute(&mut **tx)
            .await
        }
        DecisionEdit::RemoveEvidence(message) => {
            sqlx::query!(
                "delete from evidence where decision_id = $1 and message_id = $2",
                id,
                Uuid::from(message.ulid()),
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

/// [`db_err`] for deletes: the FK violation on the evidence→message
/// anchor is the "an evidenced message is undeletable" invariant doing
/// its job — a domain conflict, not a caller mistake.
fn evidence_pinned(e: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(d) = &e
        && d.code().as_deref() == Some("23503")
        && d.constraint() == Some("evidence_message_id_fkey")
    {
        return StoreError::Conflict(
            "a session here anchors evidence of a decision elsewhere — \
             evidenced messages are undeletable"
                .into(),
        );
    }
    db_err(e)
}
