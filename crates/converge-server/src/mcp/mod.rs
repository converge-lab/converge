//! The MCP surface: `/mcp`, unversioned and **stateless** — every tool call
//! is request/response only, no server-held sessions, so restarts orphan
//! nothing (per the prod decision this mirrors).
//!
//! Tool conventions:
//! - `resource_operation` names, matching the storage traits and
//!   `converge-client` — one naming scheme across the stack, and related
//!   tools cluster in the palette (`decision_add` / `decision_get` / …).
//! - Ids are ULID strings; parse failures name the offending field.
//! - Per the time-authority decision: **no datetime parameters** (instants
//!   are server-assigned), and payload instants are RFC3339 UTC only —
//!   stable and comparable, never localized or relativized server-side.
//! - Authorship is stamped server-side: the **authenticated caller**
//!   (the user the HTTP gate resolved from the bearer) working through
//!   the calling agent (`user_via_agent`), the agent ensured by natural
//!   key from MCP client info when the transport exposes it, else the
//!   generic `mcp` tool agent. The same user is every read's visibility
//!   scope — the ACL holds on this surface exactly as on REST.
//!
//! No `resolve_project` yet: project names are display-only (no natural
//! key), so resolve-by-name would be scan-then-create. Agents discover ids
//! through `project_list` until the path/alias design lands.

use std::sync::Arc;

use axum::http::request::Parts;
use converge_storage::{
    AgentKind, Author, CodeAnchor, DecisionFilter, DecisionId, DecisionStatus, GroupId, GroupKind,
    MessageId, NewAgent, NewDecision, NewGroup, NewMessage, NewProject, NewSession, Pagination,
    ProjectEdit, ProjectId, Repository, Scope, SessionId, SessionKind, SignalFilter, SignalId,
    SignalStatus, Storage, StoreError, Tier, UserId,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ErrorData as McpError, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, schemars, tool, tool_router};
use serde::{Deserialize, Serialize};

use crate::auth::Caller;

/// The `/mcp` tower service, ready to nest into the app router. `public`
/// is the deployment's external origin (`auth.public_url`), when set.
pub fn service<S: Storage + 'static>(
    store: S,
    expert: crate::expert::Expert<S>,
    public: Option<&str>,
) -> StreamableHttpService<Memory<S>, LocalSessionManager> {
    let memory = Memory::new(store, expert);
    // Stateless + plain-JSON POST responses: nothing to orphan on
    // restart, and simple JSON survives proxies better than SSE.
    let mut config = StreamableHttpServerConfig::default();
    config.stateful_mode = false;
    config.json_response = true;
    // rmcp's DNS-rebinding guard allows only the localhost family by
    // default — a deployment reached through its public name must allow
    // that name too, or every proxied request 403s on the Host header.
    if let Some(public) = public
        && let Ok(url) = url::Url::parse(public)
        && let Some(host) = url.host_str()
    {
        config.allowed_hosts.push(host.to_string());
        if let Some(port) = url.port() {
            config.allowed_hosts.push(format!("{host}:{port}"));
        }
    }
    StreamableHttpService::new(
        move || Ok(memory.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

/// The MCP server: converge memory over any storage backend.
#[derive(Clone)]
pub struct Memory<S> {
    #[allow(dead_code)] // read by the macro-generated tool dispatcher
    tool_router: ToolRouter<Self>,
    store: S,
    expert: crate::expert::Expert<S>,
}

// ---- tool wire types (ids as strings; instants never accepted) -----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DecisionAdd {
    /// The project this decision belongs to (see `project_list`).
    pub project_id: String,
    /// Short, imperative title — the line shown in lists.
    pub title: String,
    /// One-paragraph summary of what was decided.
    #[serde(default)]
    pub summary: String,
    /// Lifecycle: accepted (default), draft, proposed, or rejected.
    /// `superseded` is derived from supersession edges, never stored.
    #[serde(default)]
    pub status: Option<String>,
    /// Why the decision was needed (Markdown).
    #[serde(default)]
    pub context: Option<String>,
    /// What follows from it (Markdown).
    #[serde(default)]
    pub consequences: Option<String>,
    /// Rejected alternatives and why each lost.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// Decision ids this one replaces (creation-time supersession).
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// Message ids (from `message_add`) this decision is grounded in —
    /// the exact lines that decided it. Required here: a decision an
    /// agent records out of a conversation is only worth having if the
    /// conversation is on record. `session_ensure` once, `message_add`
    /// the exchanges, then cite their ids.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Code anchors: a line range in one file at one commit of the
    /// project's repository, with the cited lines and their sha256.
    /// The full form only — the hook completes a bare `path:lines`.
    #[serde(default)]
    pub code_evidence: Vec<CodeAnchorIn>,
    /// The exchange that produced this decision, recorded and cited in
    /// this one call. Use it when nothing else is recording the
    /// conversation for you: no `session_ensure`, no `message_add`, no
    /// ids to carry. Turns that name an `ordinal` are written once, so
    /// sending them here and syncing them later is not a duplicate.
    #[serde(default)]
    pub evidence_turns: Vec<MessageIn>,
    /// Which conversation `evidence_turns` belong to: your own stable
    /// id for this session, and a title for it. Required alongside
    /// `evidence_turns` unless the conversation is already ensured.
    #[serde(default)]
    pub conversation: Option<ConversationIn>,
}

/// The conversation a decision's inline evidence belongs to.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct ConversationIn {
    /// Your own stable reference for this conversation — a session id,
    /// a thread URL. Ensuring twice returns the same session.
    pub external: String,
    /// Human-readable, shown wherever the source is cited.
    #[serde(default)]
    pub title: Option<String>,
}

/// A code anchor on the wire, as `converge_storage::CodeAnchor`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct CodeAnchorIn {
    /// Full 40-hex commit sha.
    pub commit: String,
    /// Repository-relative path, forward slashes.
    pub path: String,
    /// `[start, end]`, 1-based, inclusive, at most 120 lines.
    pub lines: (u32, u32),
    /// The cited lines as they are at `commit`.
    pub excerpt: String,
    /// Hex sha256 of `excerpt`.
    pub digest: String,
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DecisionGet {
    pub decision_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DecisionList {
    /// Narrow to one project.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Narrow to one group (spans its projects).
    #[serde(default)]
    pub group_id: Option<String>,
    /// accepted | draft | proposed | rejected | superseded (derived).
    #[serde(default)]
    pub status: Option<String>,
    /// Only decisions you have not been shown yet, in any session.
    #[serde(default)]
    pub unseen: bool,
    /// Newest first; omit for everything.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DecisionSearch {
    /// What to find. Websearch syntax: bare words AND together, `or`
    /// alternates, `-` excludes, `"quoted phrases"` match exactly.
    pub query: String,
    /// Narrow to one project.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Narrow to one group (spans its projects).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Best matches first; omit for everything that matches.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SignalList {
    /// Signals touching this project on either end.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Signals touching this decision on either end.
    #[serde(default)]
    pub decision_id: Option<String>,
    /// proposed | confirmed | dismissed.
    #[serde(default)]
    pub status: Option<String>,
    /// watch | coordinate | conflict.
    #[serde(default)]
    pub tier: Option<String>,
    /// Only signals newer than this signal id, oldest first — to poll
    /// forward from the last one you saw.
    #[serde(default)]
    pub since: Option<String>,
    /// Only signals you have not been shown yet, in any session.
    #[serde(default)]
    pub unseen: bool,
    /// Newest first (oldest first with `since`); omit for everything.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SignalResolve {
    /// The signal to judge (see `signal_list`).
    pub signal_id: String,
    /// The verdict: `confirmed` (it holds — act on it) or `dismissed`
    /// (wrong or not worth acting on; it will NOT be raised again).
    pub status: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectList {}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SessionEnsure {
    /// The project this conversation belongs to (see `project_list`).
    pub project_id: String,
    /// Where it happens: transcript (agent session — the default),
    /// slack, pr, or incident.
    #[serde(default)]
    pub kind: Option<String>,
    /// The source system's stable reference — your own session id, a
    /// thread URL, a PR reference. Ensuring again with the same
    /// kind+external returns the same session (and refreshes the title).
    pub external: String,
    /// Human-readable title, shown wherever the source is cited.
    pub title: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MessageAdd {
    /// The session to append to (see `session_ensure`).
    pub session_id: String,
    /// Appended in order. Timestamps are server-assigned — never send them.
    pub messages: Vec<MessageIn>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MessageIn {
    /// Who said it, as displayed ("maksim", "claude").
    pub speaker: String,
    pub body: String,
    /// Where this turn sits in the conversation, counting from 0. Send
    /// it and the same turn is never recorded twice, however often it
    /// is sent; leave it out and every send appends.
    #[serde(default)]
    pub ordinal: Option<i32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GroupAdd {
    /// Group name (display; not unique).
    pub name: String,
    /// `shared` (invite members later) or `personal` (only you).
    /// REQUIRED in effect: ask the user — the kind decides who can ever
    /// see the projects inside.
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectMatch {
    /// Working directory of the session (a client-side hook injects it;
    /// omit when unknown).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Git remote URL of the working tree (hook-injected; omit when
    /// unknown).
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectBind {
    /// Bind to this existing project (from `project_match`). Exactly
    /// one of `project_id` / `name`.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Create a new project with this name and bind to it.
    #[serde(default)]
    pub name: Option<String>,
    /// Owning group for a created project; only needed when the
    /// deployment has more than one group.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Git remote URL of the working tree (hook-injected; never send).
    /// Becomes the project's repository when it has none yet.
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectDismiss {
    /// `session` = skip for now (nothing persists); `repo` = don't ask
    /// again (the client-side hook writes the opt-out marker).
    pub scope: String,
}

#[tool_router]
impl<S: Storage + 'static> Memory<S> {
    pub fn new(store: S, expert: crate::expert::Expert<S>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            store,
            expert,
        }
    }

    #[tool(description = "The full map of groups and their projects (names and \
        ids). Call this first to find the project_id the other tools need.")]
    async fn project_list(
        &self,
        Parameters(_req): Parameters<ProjectList>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let scope = self.scope(&context)?;
        let groups = self
            .store
            .group_list(scope, Pagination::default())
            .await
            .map_err(map_err)?;
        let projects = self
            .store
            .project_list(scope, Default::default(), Pagination::default())
            .await
            .map_err(map_err)?;
        let map: Vec<_> = groups
            .iter()
            .map(|g| {
                serde_json::json!({
                    "group_id": g.id,
                    "group_name": g.name,
                    "kind": g.kind,
                    "projects": projects
                        .iter()
                        .filter(|p| p.group_id == g.id)
                        .map(|p| serde_json::json!({
                            "project_id": p.id,
                            "name": p.name,
                            "description": p.description,
                        }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        json_result(&map)
    }

    #[tool(description = "Match this working tree to converge projects, \
        best candidate first (a client-side hook injects cwd + git remote). \
        Present the candidates to the user, then call `project_bind` with \
        their pick — or `project_dismiss` if they decline.")]
    // When a transport with elicitation support exists, this tool renders
    // the picker server-side and returns the outcome directly (the POC's
    // pick flow) — a capability-adaptive behavior, not a separate tool.
    async fn project_match(
        &self,
        Parameters(req): Parameters<ProjectMatch>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let scope = self.scope(&context)?;
        let groups = self
            .store
            .group_list(scope, Pagination::default())
            .await
            .map_err(map_err)?;
        let projects = self
            .store
            .project_list(scope, Default::default(), Pagination::default())
            .await
            .map_err(map_err)?;

        // The working tree's own names are the ranking hints: the repo
        // directory, and the repository name from the remote URL.
        let mut hints: Vec<String> = Vec::new();
        if let Some(cwd) = &req.cwd
            && let Some(base) = std::path::Path::new(cwd).file_name()
        {
            hints.push(base.to_string_lossy().to_lowercase());
        }
        if let Some(remote) = &req.remote
            && let Some(repo) = remote
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .rsplit(['/', ':'])
                .next()
        {
            hints.push(repo.to_lowercase());
        }
        // An exact repository match outranks every name hint: the remote
        // names the repository, the project records it.
        let remote = req.remote.as_deref().and_then(Repository::from_remote);
        let score = |name: &str, repository: Option<&Repository>| -> u8 {
            let name = name.to_lowercase();
            if remote.is_some() && repository == remote.as_ref() {
                3
            } else if hints.contains(&name) {
                2
            } else if hints
                .iter()
                .any(|h| !h.is_empty() && (h.contains(&name) || name.contains(h.as_str())))
            {
                1
            } else {
                0
            }
        };
        let mut candidates: Vec<_> = projects
            .iter()
            .map(|p| {
                let group = groups
                    .iter()
                    .find(|g| g.id == p.group_id)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                (
                    score(&p.name, p.repository.as_ref()),
                    serde_json::json!({
                        "project_id": p.id,
                        "name": p.name,
                        "description": p.description,
                        "repository": p.repository.as_ref().map(Repository::canonical),
                        "group": group,
                    }),
                )
            })
            .collect();
        candidates.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let candidates: Vec<_> = candidates.into_iter().map(|(_, c)| c).collect();
        // Groups ride along so the create path can offer a placement
        // choice without another call — placing a project decides who
        // sees it, and the same project name can exist in several groups.
        let groups: Vec<_> = groups
            .iter()
            .map(|g| {
                serde_json::json!({
                    "group_id": g.id,
                    "name": g.name,
                    "kind": format!("{:?}", g.kind).to_lowercase(),
                })
            })
            .collect();
        json_result(&serde_json::json!({
            "hints": hints,
            "candidates": candidates,
            "groups": groups,
        }))
    }

    #[tool(description = "Create a group — the visibility boundary projects \
        live in (members of a group see everything inside it). kind: \
        `shared` (others can be invited) or `personal` (only you) — ask \
        the user, never assume. Answers {group_id, name}; pass the id as \
        `group_id` to `project_bind` when creating a project in it.")]
    async fn group_add(
        &self,
        Parameters(req): Parameters<GroupAdd>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let scope = self.scope(&context)?;
        let kind = match req.kind.as_str() {
            "shared" => GroupKind::Shared,
            "personal" => GroupKind::Personal,
            other => {
                return Err(McpError::invalid_params(
                    format!("invalid kind: {other} (shared | personal)"),
                    None,
                ));
            }
        };
        let Scope::User(user) = scope else {
            return Err(McpError::invalid_params(
                "group creation needs an authenticated user caller",
                None,
            ));
        };
        let id = self
            .store
            .group_add(
                user,
                NewGroup {
                    name: req.name.clone(),
                    description: req.description,
                    kind,
                },
            )
            .await
            .map_err(map_err)?;
        json_result(&serde_json::json!({ "group_id": id, "name": req.name }))
    }

    #[tool(description = "Link the working tree to a converge project: pass \
        `project_id` for an existing one, or `name` to create it. Answers \
        {project_id, name}; a client-side hook writes the local `.converge` \
        marker from that — do NOT write the file yourself.")]
    async fn project_bind(
        &self,
        Parameters(req): Parameters<ProjectBind>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let scope = self.scope(&context)?;
        let remote = req.remote.as_deref().and_then(Repository::from_remote);
        let (id, name) = match (req.project_id.as_deref(), req.name) {
            (Some(id), None) => {
                let id: ProjectId = parse_id(id, "project_id")?;
                let project = self
                    .store
                    .project_get(scope, id)
                    .await
                    .map_err(map_err)?
                    .ok_or_else(|| McpError::invalid_params("unknown project_id", None))?;
                // First bind from a working tree records where the code
                // lives; a later bind never overwrites what is set.
                if project.repository.is_none()
                    && let Some(repository) = remote.clone()
                {
                    self.store
                        .project_edit(
                            scope,
                            id,
                            vec![ProjectEdit::SetRepository(Some(repository))],
                        )
                        .await
                        .map_err(map_err)?;
                }
                (id, project.name)
            }
            (None, Some(name)) => {
                let groups = self
                    .store
                    .group_list(scope, Pagination::default())
                    .await
                    .map_err(map_err)?;
                // Group placement is a visibility decision (membership IS
                // visibility), so it defaults silently only when that's
                // harmless: a sole *personal* group. A shared group is
                // never a silent target — the parallax lesson: a solo
                // project auto-placed into the one (shared) group became
                // visible to every member without anyone choosing that.
                let group = match (req.group_id.as_deref(), groups.as_slice()) {
                    (Some(gid), _) => parse_id::<GroupId>(gid, "group_id")?,
                    (None, [only]) if only.kind == GroupKind::Personal => only.id,
                    (None, _) => {
                        let list: Vec<String> = groups
                            .iter()
                            .map(|g| format!("{} ({:?}) = {}", g.name, g.kind, g.id))
                            .collect();
                        return Err(McpError::invalid_params(
                            format!(
                                "pass group_id — placing a project decides who can \
                                 see it. Existing: {}; or `group_add` a new one and \
                                 ask the user which they want",
                                list.join(", ")
                            ),
                            None,
                        ));
                    }
                };
                let id = self
                    .store
                    .project_add(
                        scope,
                        NewProject {
                            group_id: group,
                            name: name.clone(),
                            description: None,
                            repository: remote.clone(),
                        },
                    )
                    .await
                    .map_err(map_err)?;
                (id, name)
            }
            _ => {
                return Err(McpError::invalid_params(
                    "pass exactly one of project_id (bind existing) or name (create)",
                    None,
                ));
            }
        };
        json_result(&serde_json::json!({ "project_id": id, "name": name }))
    }

    #[tool(description = "The user declined to link this repo. scope=session \
        = skip for now (nothing persists); scope=repo = don't ask again (a \
        client-side hook writes the opt-out marker).")]
    async fn project_dismiss(
        &self,
        Parameters(req): Parameters<ProjectDismiss>,
    ) -> Result<CallToolResult, McpError> {
        match req.scope.as_str() {
            "session" | "repo" => json_result(
                &serde_json::json!({ "dismissed": req.scope, "disable": req.scope == "repo" }),
            ),
            other => Err(McpError::invalid_params(
                format!("invalid scope: {other} (session | repo)"),
                None,
            )),
        }
    }

    #[tool(description = "Ensure the conversation you're working in exists as \
        a session — call once, early, with a stable external reference (your \
        own session id). Idempotent: the same kind+external always returns the \
        same session_id, which message_add and decision evidence need.")]
    async fn session_ensure(
        &self,
        Parameters(req): Parameters<SessionEnsure>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let project_id: ProjectId = parse_id(&req.project_id, "project_id")?;
        let kind = match req.kind.as_deref() {
            None => SessionKind::Transcript,
            Some(s) => parse_session_kind(s)?,
        };
        let id = self
            .store
            .session_ensure(
                self.scope(&context)?,
                NewSession {
                    project_id,
                    kind,
                    external: req.external,
                    title: req.title,
                },
            )
            .await
            .map_err(map_err)?;
        let scope = self.scope(&context)?;
        let next = self
            .store
            .message_next_ordinal(scope, id)
            .await
            .map_err(map_err)?;
        let archive = archives(&self.store, scope, project_id).await?;
        json_result(&serde_json::json!({
            "session_id": id,
            "next_ordinal": next,
            "archive_transcripts": archive,
        }))
    }

    #[tool(description = "Append messages to a session's stream, in order — \
        record the conversation as it happens. Returns the new message ids; \
        pass them as `evidence` on decision_add to anchor the exact lines \
        that decided it.")]
    async fn message_add(
        &self,
        Parameters(req): Parameters<MessageAdd>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let session: SessionId = parse_id(&req.session_id, "session_id")?;
        let messages = req
            .messages
            .into_iter()
            .map(|m| NewMessage {
                speaker: m.speaker,
                body: m.body,
                // Live recording: capture time is the server's to assign
                // (the time-authority decision); importers with real
                // external timestamps use the REST batch surface instead.
                sent_at: None,
                ordinal: m.ordinal,
            })
            .collect();
        let scope = self.scope(&context)?;
        let project = self
            .store
            .session_get(scope, session)
            .await
            .map_err(map_err)?
            .ok_or_else(|| McpError::invalid_params("unknown session_id", None))?
            .project_id;
        if !archives(&self.store, scope, project).await? {
            return Err(McpError::invalid_params(
                "this project records only the turns a decision cites, not whole \
                 conversations — send them as `evidence_turns` on `decision_add`",
                None,
            ));
        }
        let ids = self
            .store
            .message_add(scope, session, messages)
            .await
            .map_err(map_err)?;
        crate::metrics::evidence_messages("mcp", ids.len());
        json_result(&serde_json::json!({ "message_ids": ids }))
    }

    #[tool(description = "Record a decision (ADR): what was decided, why, what \
        was rejected. Set `supersedes` when it replaces earlier decisions. \
        Authorship and timestamps are recorded server-side — never send them.")]
    async fn decision_add(
        &self,
        Parameters(req): Parameters<DecisionAdd>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let project_id: ProjectId = parse_id(&req.project_id, "project_id")?;
        let status = match req.status.as_deref() {
            None => DecisionStatus::Accepted,
            Some(s) => parse_status(s)?,
        };
        let supersedes = req
            .supersedes
            .iter()
            .map(|s| parse_id::<DecisionId>(s, "supersedes"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut evidence = req
            .evidence
            .iter()
            .map(|m| parse_id::<MessageId>(m, "evidence"))
            .collect::<Result<Vec<_>, _>>()?;
        // The exchange, recorded here and cited: for a caller with
        // nothing else putting the conversation on record. Turns that
        // name their position are written once, so a hook or a later
        // sync sending the same ones is not a second copy.
        if !req.evidence_turns.is_empty() {
            let Some(conversation) = &req.conversation else {
                return Err(McpError::invalid_params(
                    "evidence_turns needs `conversation` — your own stable id for this \
                     conversation, and a title",
                    None,
                ));
            };
            let session = self
                .store
                .session_ensure(
                    self.scope(&context)?,
                    NewSession {
                        project_id,
                        kind: SessionKind::Transcript,
                        external: conversation.external.clone(),
                        title: conversation
                            .title
                            .clone()
                            .unwrap_or_else(|| conversation.external.clone()),
                    },
                )
                .await
                .map_err(map_err)?;
            let turns: Vec<NewMessage> = req
                .evidence_turns
                .iter()
                .map(|m| NewMessage {
                    speaker: m.speaker.clone(),
                    body: m.body.clone(),
                    sent_at: None,
                    ordinal: m.ordinal,
                })
                .collect();
            let recorded = self
                .store
                .message_add(self.scope(&context)?, session, turns)
                .await
                .map_err(map_err)?;
            evidence.extend(recorded);
            evidence.sort_unstable();
            evidence.dedup();
        }
        // This door is the agent's. A person typing a decision into the
        // web is their own source; an agent recording one out of a
        // conversation has to put the conversation on record first, or
        // "verifiable" is a word in the instructions and nothing else.
        if evidence.is_empty() && req.code_evidence.is_empty() {
            return Err(McpError::invalid_params(
                "evidence is required: `session_ensure` this conversation, \
                 `message_add` the exchanges that decided it, and pass their \
                 message ids as `evidence` — or cite committed code as \
                 `code_evidence`",
                None,
            ));
        }

        // Authorship: the deployment user working through the calling
        // agent (see `caller`); the same user is the write's scope.
        let author = self.caller(&context).await?;
        let scope = match author {
            Author::UserViaAgent { user, .. } | Author::User(user) => Scope::User(user),
            Author::Agent(_) => Scope::System,
        };

        let id = self
            .store
            .decision_add(
                scope,
                NewDecision {
                    project_id,
                    status,
                    title: req.title,
                    summary: req.summary,
                    context: req.context,
                    consequences: req.consequences,
                    alternatives: req
                        .alternatives
                        .into_iter()
                        .map(|a| converge_storage::Alternative {
                            option: a.option,
                            why_rejected: a.why_rejected,
                        })
                        .collect(),
                    authors: vec![author],
                    supersedes,
                    evidence,
                    code_evidence: req
                        .code_evidence
                        .into_iter()
                        .map(|a| CodeAnchor {
                            commit: a.commit,
                            path: a.path,
                            lines: a.lines,
                            excerpt: a.excerpt,
                            digest: a.digest,
                        })
                        .collect(),
                },
            )
            .await
            .map_err(map_err)?;
        crate::metrics::decision_recorded("mcp");
        self.expert.detect(id);
        // Prevention: same-project near-matches ride the tool result so
        // the agent can raise "supersede instead?" while the author still
        // holds the intent.
        let similar: Vec<_> = self
            .expert
            .similar(id)
            .await
            .into_iter()
            .map(|(id, title)| serde_json::json!({ "decision_id": id, "title": title }))
            .collect();
        if similar.is_empty() {
            json_result(&serde_json::json!({ "decision_id": id }))
        } else {
            json_result(&serde_json::json!({
                "decision_id": id,
                "similar": similar,
                "note": "existing decisions in this project overlap this one — \
                         if it replaces one of them, `decision_edit` it with \
                         `supersedes`; surface the overlap to the user",
            }))
        }
    }

    #[tool(description = "Get a decision by id: the full ADR, its authors, \
        and its graph edges (supersession chain, cross-references).")]
    async fn decision_get(
        &self,
        Parameters(req): Parameters<DecisionGet>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let id: DecisionId = parse_id(&req.decision_id, "decision_id")?;
        let scope = self.scope(&context)?;
        let decision = self
            .store
            .decision_get(scope, id)
            .await
            .map_err(map_err)?
            .ok_or_else(|| McpError::invalid_params("decision not found", None))?;
        let edges = self
            .store
            .decision_edges(scope, id)
            .await
            .map_err(map_err)?
            .unwrap_or_default();
        json_result(&serde_json::json!({ "decision": decision, "edges": edges }))
    }

    #[tool(description = "List decisions, newest first. Filter by project, \
        group, or status; `superseded` matches the derived status.")]
    async fn decision_list(
        &self,
        Parameters(req): Parameters<DecisionList>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let filter = DecisionFilter {
            project: req
                .project_id
                .as_deref()
                .map(|s| parse_id::<ProjectId>(s, "project_id"))
                .transpose()?,
            group: req
                .group_id
                .as_deref()
                .map(|s| parse_id::<GroupId>(s, "group_id"))
                .transpose()?,
            status: req.status.as_deref().map(parse_status).transpose()?,
            unseen: req.unseen,
        };
        let page = Pagination {
            limit: req.limit,
            cursor: None,
        };
        let decisions = self
            .store
            .decision_list(self.scope(&context)?, filter, page)
            .await
            .map_err(map_err)?;
        let items: Vec<_> = decisions
            .iter()
            .map(|d| {
                serde_json::json!({
                    "decision_id": d.id,
                    "project_id": d.project_id,
                    "status": d.status,
                    "title": d.title,
                    "summary": d.summary,
                    // `json!` would use time's default (array) encoding —
                    // the rfc3339 serde attribute lives on the Decision
                    // struct, not the type. Format explicitly.
                    "captured_at": d
                        .captured_at
                        .format(&time::format_description::well_known::Rfc3339)
                        .expect("timestamps format as RFC3339"),
                })
            })
            .collect();
        json_result(&items)
    }

    #[tool(description = "Full-text search over decisions, best match \
        first (title weighs over summary over body). Websearch syntax: \
        bare words AND, `or`, `-` excludes, \"quoted phrases\". Use this \
        before decision_list when looking for a topic rather than \
        browsing.")]
    async fn decision_search(
        &self,
        Parameters(req): Parameters<DecisionSearch>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let filter = DecisionFilter {
            project: req
                .project_id
                .as_deref()
                .map(|s| parse_id::<ProjectId>(s, "project_id"))
                .transpose()?,
            group: req
                .group_id
                .as_deref()
                .map(|s| parse_id::<GroupId>(s, "group_id"))
                .transpose()?,
            status: None,
            unseen: false,
        };
        let decisions = self
            .store
            .decision_search(self.scope(&context)?, &req.query, filter, req.limit)
            .await
            .map_err(map_err)?;
        let items: Vec<_> = decisions
            .iter()
            .map(|d| {
                serde_json::json!({
                    "decision_id": d.id,
                    "project_id": d.project_id,
                    "status": d.status,
                    "title": d.title,
                    "summary": d.summary,
                })
            })
            .collect();
        json_result(&items)
    }

    #[tool(description = "List signals — observations that one decision \
        affects others (tier: watch < coordinate < conflict; status: \
        proposed = awaiting judgment). project_id/decision_id match either \
        end. Surface proposed signals to the user, then `signal_resolve` \
        with their verdict.")]
    async fn signal_list(
        &self,
        Parameters(req): Parameters<SignalList>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let filter = SignalFilter {
            project: req
                .project_id
                .as_deref()
                .map(|s| parse_id::<ProjectId>(s, "project_id"))
                .transpose()?,
            decision: req
                .decision_id
                .as_deref()
                .map(|s| parse_id::<DecisionId>(s, "decision_id"))
                .transpose()?,
            status: req.status.as_deref().map(parse_signal_status).transpose()?,
            tier: req.tier.as_deref().map(parse_tier).transpose()?,
            since: req
                .since
                .as_deref()
                .map(|s| parse_id::<SignalId>(s, "since"))
                .transpose()?,
            unseen: req.unseen,
        };
        let signals = self
            .store
            .signal_list(
                self.scope(&context)?,
                filter,
                Pagination {
                    limit: req.limit,
                    cursor: None,
                },
            )
            .await
            .map_err(map_err)?;
        let items: Vec<_> = signals
            .iter()
            .map(|s| {
                serde_json::json!({
                    "signal_id": s.id,
                    "source": s.source,
                    "targets": s.targets,
                    "kind": s.kind,
                    "tier": s.tier,
                    "status": s.status,
                    "title": s.title,
                    "text": s.text,
                    "consequence": s.consequence,
                    "recommendation": s.recommendation,
                })
            })
            .collect();
        json_result(&items)
    }

    #[tool(description = "Resolve a signal with the user's verdict: \
        `confirmed` (the observation holds — act on it) or `dismissed` \
        (wrong or not worth acting on — it will not be raised again). \
        Ask the user before resolving; never judge on their behalf.")]
    async fn signal_resolve(
        &self,
        Parameters(req): Parameters<SignalResolve>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let id: SignalId = parse_id(&req.signal_id, "signal_id")?;
        let status = parse_signal_status(&req.status)?;
        let by = self.caller(&context).await?;
        let scope = match by {
            Author::UserViaAgent { user, .. } | Author::User(user) => Scope::User(user),
            Author::Agent(_) => Scope::System,
        };
        self.store
            .signal_resolve(scope, id, status, by)
            .await
            .map_err(map_err)?;
        json_result(&serde_json::json!({ "signal_id": id, "status": status }))
    }

    /// The caller's visibility scope: the authenticated user behind this
    /// very request — same ACL as REST, per credential, never a fixture.
    fn scope(&self, context: &RequestContext<RoleServer>) -> Result<Scope, McpError> {
        Ok(Scope::User(user(context)?))
    }

    /// The judging/authoring identity: the authenticated user working
    /// through the calling agent (client info when the transport carries
    /// it, the generic tool agent otherwise).
    async fn caller(&self, context: &RequestContext<RoleServer>) -> Result<Author, McpError> {
        let user = user(context)?;
        let client = context
            .peer
            .peer_info()
            .map(|info| info.client_info.name.clone())
            .unwrap_or_else(|| "mcp".into());
        let agent = self
            .store
            .agent_ensure(NewAgent {
                kind: AgentKind::Tool,
                name: client,
            })
            .await
            .map_err(map_err)?;
        Ok(Author::UserViaAgent { user, agent })
    }
}

/// The authenticated user on this request: the HTTP gate verifies the
/// bearer and parks [`Caller`] in the request extensions; rmcp forwards
/// them into the tool context as `http::request::Parts`. Absence is an
/// internal error — `/mcp` is unreachable unauthenticated.
fn user(context: &RequestContext<RoleServer>) -> Result<UserId, McpError> {
    context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .map(|caller| caller.user)
        .ok_or_else(|| {
            tracing::error!("mcp request reached a tool without an authenticated caller");
            McpError::internal_error("no authenticated caller on the request", None)
        })
}

impl<S: Storage + 'static> ServerHandler for Memory<S> {
    // What `#[tool_handler]` would generate, plus one histogram per call
    // — the agent's view of this server, by tool.
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, McpError> {
        Ok(rmcp::model::ListToolsResult {
            tools: self.tool_router.list_all(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = crate::metrics::tool_label(&request.name);
        let started = std::time::Instant::now();
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(call).await;
        // rmcp turns an argument mismatch into a successful transport
        // reply that carries `is_error` — the failure most worth seeing.
        let ok = result.as_ref().is_ok_and(|r| r.is_error != Some(true));
        crate::metrics::tool_call(tool, ok, started);
        result
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned()
    }

    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::from_build_env();
        info.instructions = Some(
            "Converge: shared decision memory. Call `project_list` to \
             find project ids, `decision_add` after a design decision \
             lands (set `supersedes` when it replaces one), and \
             `decision_list`/`decision_get` before re-deciding \
             something that may already be settled. Decisions are \
             verifiable, so `decision_add` requires evidence: message ids \
             from `message_add` (`session_ensure` this conversation once, \
             record the exchanges as they happen) or committed code as \
             `code_evidence`. A converge hook fills both in where one is \
             installed."
                .into(),
        );
        info
    }

    /// Default behavior plus a capability probe: we're deciding when to
    /// adopt the sessionless spec's task-based elicitation (SEP-2322),
    /// and the deciding fact is what real clients declare — this log
    /// answers it per connect (`tasks`/`elicitation` presence).
    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, McpError> {
        tracing::info!(
            client = %request.client_info.name,
            version = %request.protocol_version,
            capabilities = %serde_json::to_string(&request.capabilities).unwrap_or_default(),
            "mcp client connected"
        );
        context.peer.set_peer_info(request);
        Ok(self.get_info())
    }
}

// ---- shared plumbing -------------------------------------------------------

/// Does this project keep whole conversations, or only the turns a
/// decision cites? A project that has gone missing under the caller
/// keeps nothing.
async fn archives<S: Storage>(
    store: &S,
    scope: Scope,
    project: ProjectId,
) -> Result<bool, McpError> {
    Ok(store
        .project_get(scope, project)
        .await
        .map_err(map_err)?
        .is_some_and(|p| p.archive_transcripts))
}

fn json_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("serialize response: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn parse_id<T: From<ulid::Ulid>>(s: &str, field: &str) -> Result<T, McpError> {
    s.parse::<ulid::Ulid>()
        .map(T::from)
        .map_err(|_| McpError::invalid_params(format!("invalid {field}: {s}"), None))
}

fn parse_session_kind(s: &str) -> Result<SessionKind, McpError> {
    match s {
        "transcript" => Ok(SessionKind::Transcript),
        "slack" => Ok(SessionKind::Slack),
        "pr" => Ok(SessionKind::Pr),
        "incident" => Ok(SessionKind::Incident),
        other => Err(McpError::invalid_params(
            format!("invalid kind: {other} (transcript | slack | pr | incident)"),
            None,
        )),
    }
}

fn parse_status(s: &str) -> Result<DecisionStatus, McpError> {
    match s {
        "accepted" => Ok(DecisionStatus::Accepted),
        "draft" => Ok(DecisionStatus::Draft),
        "proposed" => Ok(DecisionStatus::Proposed),
        "superseded" => Ok(DecisionStatus::Superseded),
        "rejected" => Ok(DecisionStatus::Rejected),
        other => Err(McpError::invalid_params(
            format!("invalid status: {other}"),
            None,
        )),
    }
}

fn parse_signal_status(s: &str) -> Result<SignalStatus, McpError> {
    match s {
        "proposed" => Ok(SignalStatus::Proposed),
        "confirmed" => Ok(SignalStatus::Confirmed),
        "dismissed" => Ok(SignalStatus::Dismissed),
        other => Err(McpError::invalid_params(
            format!("invalid status: {other} (proposed | confirmed | dismissed)"),
            None,
        )),
    }
}

fn parse_tier(s: &str) -> Result<Tier, McpError> {
    match s {
        "watch" => Ok(Tier::Watch),
        "coordinate" => Ok(Tier::Coordinate),
        "conflict" => Ok(Tier::Conflict),
        other => Err(McpError::invalid_params(
            format!("invalid tier: {other} (watch | coordinate | conflict)"),
            None,
        )),
    }
}

/// [`StoreError`] → MCP error codes: caller mistakes are invalid-params,
/// backend trouble is internal (details logged, not leaked).
fn map_err(e: StoreError) -> McpError {
    match e {
        StoreError::NotFound => McpError::invalid_params("not found", None),
        StoreError::Invalid(m) => McpError::invalid_params(m, None),
        StoreError::Conflict(m) => McpError::invalid_params(m, None),
        // Tools sit behind the auth gate; storage never returns this.
        StoreError::Unauthorized => McpError::invalid_params("unauthorized", None),
        StoreError::Unavailable(_) | StoreError::Backend(_) => {
            tracing::error!(error = %e, "storage failure in mcp tool");
            McpError::internal_error("storage failure", None)
        }
    }
}

#[cfg(test)]
mod metric_labels {
    use super::Memory;

    /// Every tool the router has must have its own metric label: a new
    /// `#[tool]` without an arm in `metrics::tool_label` would be timed
    /// as `other`, silently.
    #[test]
    fn every_tool_has_a_label() {
        for tool in Memory::<converge_storage_postgres::PgStorage>::tool_router().list_all() {
            assert_eq!(
                crate::metrics::tool_label(&tool.name),
                tool.name,
                "tool {} is not in metrics::tool_label",
                tool.name
            );
        }
    }
}
