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
//! - Authorship is stamped server-side: the deployment user working
//!   through the calling agent (`user_via_agent`), the agent ensured by
//!   natural key from MCP client info when the transport exposes it, else
//!   the generic `mcp` tool agent.
//!
//! No `resolve_project` yet: project names are display-only (no natural
//! key), so resolve-by-name would be scan-then-create. Agents discover ids
//! through `project_list` until the path/alias design lands.

use std::sync::Arc;

use converge_storage::{
    AgentKind, Author, DecisionFilter, DecisionId, DecisionStatus, GroupId, Identity, MessageId,
    NewAgent, NewDecision, NewMessage, NewProject, NewSession, Pagination, ProjectId, SessionId,
    SessionKind, Storage, StoreError,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ErrorData as McpError, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

/// The `/mcp` tower service, ready to nest into the app router.
pub fn service<S: Storage + 'static>(
    store: S,
    me: Identity,
) -> StreamableHttpService<Memory<S>, LocalSessionManager> {
    let memory = Memory::new(store, me);
    // Stateless + plain-JSON POST responses: nothing to orphan on
    // restart, and simple JSON survives proxies better than SSE.
    let mut config = StreamableHttpServerConfig::default();
    config.stateful_mode = false;
    config.json_response = true;
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
    me: Identity,
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
    /// anchor the exact lines that decided it.
    #[serde(default)]
    pub evidence: Vec<String>,
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
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProjectDismiss {
    /// `session` = skip for now (nothing persists); `repo` = don't ask
    /// again (the client-side hook writes the opt-out marker).
    pub scope: String,
}

#[tool_router]
impl<S: Storage + 'static> Memory<S> {
    pub fn new(store: S, me: Identity) -> Self {
        Self {
            tool_router: Self::tool_router(),
            store,
            me,
        }
    }

    #[tool(description = "The full map of groups and their projects (names and \
        ids). Call this first to find the project_id the other tools need.")]
    async fn project_list(
        &self,
        Parameters(_req): Parameters<ProjectList>,
    ) -> Result<CallToolResult, McpError> {
        let groups = self
            .store
            .group_list(Pagination::default())
            .await
            .map_err(map_err)?;
        let projects = self
            .store
            .project_list(Default::default(), Pagination::default())
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
    ) -> Result<CallToolResult, McpError> {
        let groups = self
            .store
            .group_list(Pagination::default())
            .await
            .map_err(map_err)?;
        let projects = self
            .store
            .project_list(Default::default(), Pagination::default())
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
        let score = |name: &str| -> u8 {
            let name = name.to_lowercase();
            if hints.contains(&name) {
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
                    score(&p.name),
                    serde_json::json!({
                        "project_id": p.id,
                        "name": p.name,
                        "description": p.description,
                        "group": group,
                    }),
                )
            })
            .collect();
        candidates.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let candidates: Vec<_> = candidates.into_iter().map(|(_, c)| c).collect();
        json_result(&serde_json::json!({ "hints": hints, "candidates": candidates }))
    }

    #[tool(description = "Link the working tree to a converge project: pass \
        `project_id` for an existing one, or `name` to create it. Answers \
        {project_id, name}; a client-side hook writes the local `.converge` \
        marker from that — do NOT write the file yourself.")]
    async fn project_bind(
        &self,
        Parameters(req): Parameters<ProjectBind>,
    ) -> Result<CallToolResult, McpError> {
        let (id, name) = match (req.project_id.as_deref(), req.name) {
            (Some(id), None) => {
                let id: ProjectId = parse_id(id, "project_id")?;
                let project = self
                    .store
                    .project_get(id)
                    .await
                    .map_err(map_err)?
                    .ok_or_else(|| McpError::invalid_params("unknown project_id", None))?;
                (id, project.name)
            }
            (None, Some(name)) => {
                let groups = self
                    .store
                    .group_list(Pagination::default())
                    .await
                    .map_err(map_err)?;
                let group = match (req.group_id.as_deref(), groups.len()) {
                    (Some(gid), _) => parse_id::<GroupId>(gid, "group_id")?,
                    (None, 1) => groups[0].id,
                    (None, _) => {
                        let list: Vec<String> = groups
                            .iter()
                            .map(|g| format!("{} = {}", g.name, g.id))
                            .collect();
                        return Err(McpError::invalid_params(
                            format!("several groups exist; pass group_id ({})", list.join(", ")),
                            None,
                        ));
                    }
                };
                let id = self
                    .store
                    .project_add(NewProject {
                        group_id: group,
                        name: name.clone(),
                        description: None,
                    })
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
    ) -> Result<CallToolResult, McpError> {
        let project_id: ProjectId = parse_id(&req.project_id, "project_id")?;
        let kind = match req.kind.as_deref() {
            None => SessionKind::Transcript,
            Some(s) => parse_session_kind(s)?,
        };
        let id = self
            .store
            .session_ensure(NewSession {
                project_id,
                kind,
                external: req.external,
                title: req.title,
            })
            .await
            .map_err(map_err)?;
        json_result(&serde_json::json!({ "session_id": id }))
    }

    #[tool(description = "Append messages to a session's stream, in order — \
        record the conversation as it happens. Returns the new message ids; \
        pass them as `evidence` on decision_add to anchor the exact lines \
        that decided it.")]
    async fn message_add(
        &self,
        Parameters(req): Parameters<MessageAdd>,
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
            })
            .collect();
        let ids = self
            .store
            .message_add(session, messages)
            .await
            .map_err(map_err)?;
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
        let evidence = req
            .evidence
            .iter()
            .map(|m| parse_id::<MessageId>(m, "evidence"))
            .collect::<Result<Vec<_>, _>>()?;

        // Authorship: the deployment user working through the calling
        // agent. Client info is the best identity the transport offers;
        // stateless requests may not carry it — then the generic tool
        // agent stands in.
        let user = self
            .store
            .user_login(self.me.clone())
            .await
            .map_err(map_err)?;
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

        let id = self
            .store
            .decision_add(NewDecision {
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
                authors: vec![Author::UserViaAgent { user, agent }],
                supersedes,
                evidence,
            })
            .await
            .map_err(map_err)?;
        json_result(&serde_json::json!({ "decision_id": id }))
    }

    #[tool(description = "Get a decision by id: the full ADR, its authors, \
        and its graph edges (supersession chain, cross-references).")]
    async fn decision_get(
        &self,
        Parameters(req): Parameters<DecisionGet>,
    ) -> Result<CallToolResult, McpError> {
        let id: DecisionId = parse_id(&req.decision_id, "decision_id")?;
        let decision = self
            .store
            .decision_get(id)
            .await
            .map_err(map_err)?
            .ok_or_else(|| McpError::invalid_params("decision not found", None))?;
        let edges = self
            .store
            .decision_edges(id)
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
        };
        let page = Pagination {
            limit: req.limit,
            cursor: None,
        };
        let decisions = self
            .store
            .decision_list(filter, page)
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
        };
        let decisions = self
            .store
            .decision_search(&req.query, filter, req.limit)
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
}

#[tool_handler]
impl<S: Storage + 'static> ServerHandler for Memory<S> {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::from_build_env();
        info.instructions = Some(
            "Converge: shared decision memory. Call `project_list` to \
             find project ids, `decision_add` after a design decision \
             lands (set `supersedes` when it replaces one), and \
             `decision_list`/`decision_get` before re-deciding \
             something that may already be settled. To make decisions \
             verifiable, `session_ensure` this conversation once, \
             `message_add` the exchanges as they happen, and anchor \
             `decision_add` with `evidence` message ids — the exact \
             lines that decided it."
                .into(),
        );
        info
    }
}

// ---- shared plumbing -------------------------------------------------------

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
