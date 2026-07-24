//! Typed HTTP client for the Converge API.
//!
//! The wire contract *is* the domain: requests and responses are the
//! `converge-storage` types the server serves, so a contract change is a
//! compile error here, not a runtime surprise. Errors map back onto
//! [`StoreError`] (the envelope's `code` is the variant; transport
//! failures read as `Unavailable`), keeping the seam uniform for every
//! consumer — the web UI (wasm: reqwest rides the browser's fetch) and the
//! future CLI (native: rustls) share this one client.

// The client's public API is complete on its own: every type its methods
// mention is re-exported, so consumers (the web UI, the future CLI) depend
// on this crate alone and never name the storage crate.
pub use converge_storage::{
    Agent, AgentId, AgentKind, Alternative, AuthInfo, Author, Decision, DecisionEdit,
    DecisionFilter, DecisionId, DecisionStatus, Edges, Group, GroupEdit, GroupId, GroupKind,
    Identity, Message, MessageId, Minted, NewAgent, NewDecision, NewGroup, NewMessage, NewProject,
    NewSession, NewSignal, NewToken, Page, Pagination, Project, ProjectEdit, ProjectFilter,
    ProjectId, Related, Session, SessionFilter, SessionId, SessionKind, Signal, SignalFilter,
    SignalId, SignalStatus, Source, StoreError, Tier, Token, TokenId, User, UserId,
};
use reqwest::{Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

/// A Converge API client, addressed at the server's origin
/// (e.g. `http://127.0.0.1:8080` — the client owns the `/api/v1` layout).
/// Cheap to clone.
#[derive(Debug, Clone)]
pub struct Client {
    base: Url,
    token: Option<String>,
    http: reqwest::Client,
}

#[derive(serde::Deserialize)]
struct Envelope {
    error: Reason,
}

#[derive(serde::Deserialize)]
struct Reason {
    code: String,
    message: String,
}

#[derive(serde::Deserialize)]
struct Created<Id> {
    id: Id,
}

#[derive(Serialize)]
struct Login<'a> {
    token: &'a str,
}

impl Client {
    pub fn new(base: Url) -> Self {
        Self {
            base,
            token: None,
            http: reqwest::Client::new(),
        }
    }

    /// Authenticate every request with this bearer token. The server is
    /// always-on auth; only the browser (session cookie) goes without.
    pub fn with_token(self, token: impl Into<String>) -> Self {
        Self {
            token: Some(token.into()),
            ..self
        }
    }

    /// The origin this client is addressed at.
    pub fn base(&self) -> &Url {
        &self.base
    }

    fn authed(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    // Groups

    pub async fn group_add(&self, new: &NewGroup) -> Result<GroupId, StoreError> {
        self.create("groups", new).await
    }

    pub async fn group_get(&self, id: GroupId) -> Result<Option<Group>, StoreError> {
        self.fetch(&format!("groups/{id}")).await
    }

    pub async fn group_list(&self, page: &Pagination<GroupId>) -> Result<Page<Group>, StoreError> {
        self.list("groups", &(), page).await
    }

    pub async fn group_edit(&self, id: GroupId, edits: &[GroupEdit]) -> Result<(), StoreError> {
        self.apply(&format!("groups/{id}"), edits).await
    }

    // Projects

    pub async fn project_add(&self, new: &NewProject) -> Result<ProjectId, StoreError> {
        self.create("projects", new).await
    }

    pub async fn project_get(&self, id: ProjectId) -> Result<Option<Project>, StoreError> {
        self.fetch(&format!("projects/{id}")).await
    }

    pub async fn project_list(
        &self,
        filter: &ProjectFilter,
        page: &Pagination<ProjectId>,
    ) -> Result<Page<Project>, StoreError> {
        self.list("projects", filter, page).await
    }

    /// The `/groups/{id}/projects` relation projection: an unknown group is
    /// `NotFound`, unlike the flat filter which would answer with an empty
    /// page.
    pub async fn group_projects(
        &self,
        group: GroupId,
        page: &Pagination<ProjectId>,
    ) -> Result<Page<Project>, StoreError> {
        self.list(&format!("groups/{group}/projects"), &(), page)
            .await
    }

    pub async fn project_edit(
        &self,
        id: ProjectId,
        edits: &[ProjectEdit],
    ) -> Result<(), StoreError> {
        self.apply(&format!("projects/{id}"), edits).await
    }

    // Decisions

    pub async fn decision_add(&self, new: &NewDecision) -> Result<DecisionId, StoreError> {
        self.create("decisions", new).await
    }

    pub async fn decision_get(&self, id: DecisionId) -> Result<Option<Decision>, StoreError> {
        self.fetch(&format!("decisions/{id}")).await
    }

    pub async fn decision_list(
        &self,
        filter: &DecisionFilter,
        page: &Pagination<DecisionId>,
    ) -> Result<Page<Decision>, StoreError> {
        self.list("decisions", filter, page).await
    }

    /// Ranked full-text search over decisions (`?q=`, websearch syntax:
    /// bare words AND, `or`, `-`, `"quoted phrases"`) — best match
    /// first, unpaged.
    pub async fn decision_search(
        &self,
        query: &str,
        filter: &DecisionFilter,
        limit: Option<u32>,
    ) -> Result<Vec<Decision>, StoreError> {
        #[derive(Serialize)]
        struct Q<'a> {
            q: &'a str,
            limit: Option<u32>,
        }
        Ok(self
            .list("decisions", filter, &Q { q: query, limit })
            .await?
            .items)
    }

    /// One project's decision log (`/projects/{id}/decisions`). The filter's
    /// `project`/`group` must stay unset — the path binds them.
    pub async fn project_decisions(
        &self,
        project: ProjectId,
        filter: &DecisionFilter,
        page: &Pagination<DecisionId>,
    ) -> Result<Page<Decision>, StoreError> {
        self.list(&format!("projects/{project}/decisions"), filter, page)
            .await
    }

    /// The group-wide feed (`/groups/{id}/decisions`), spanning the group's
    /// projects; `filter.project` narrows within it, `filter.group` must
    /// stay unset — the path binds it.
    pub async fn group_decisions(
        &self,
        group: GroupId,
        filter: &DecisionFilter,
        page: &Pagination<DecisionId>,
    ) -> Result<Page<Decision>, StoreError> {
        self.list(&format!("groups/{group}/decisions"), filter, page)
            .await
    }

    pub async fn decision_edit(
        &self,
        id: DecisionId,
        edits: &[DecisionEdit],
    ) -> Result<(), StoreError> {
        self.apply(&format!("decisions/{id}"), edits).await
    }

    /// The one-hop graph neighbourhood, both directions.
    pub async fn decision_edges(&self, id: DecisionId) -> Result<Option<Edges>, StoreError> {
        self.fetch(&format!("decisions/{id}/edges")).await
    }

    /// The cited excerpts: sessions with anchored messages + context.
    pub async fn decision_sources(
        &self,
        id: DecisionId,
    ) -> Result<Option<Vec<Source>>, StoreError> {
        self.fetch(&format!("decisions/{id}/sources")).await
    }

    // Sessions + message streams (evidence)

    /// Create-or-refresh by the `(kind, external)` natural key.
    pub async fn session_ensure(&self, new: &NewSession) -> Result<SessionId, StoreError> {
        self.create("sessions", new).await
    }

    pub async fn session_get(&self, id: SessionId) -> Result<Option<Session>, StoreError> {
        self.fetch(&format!("sessions/{id}")).await
    }

    pub async fn session_list(
        &self,
        filter: &SessionFilter,
        page: &Pagination<SessionId>,
    ) -> Result<Page<Session>, StoreError> {
        self.list("sessions", filter, page).await
    }

    /// Append a batch to the stream; answers the new ids, in order.
    pub async fn message_add(
        &self,
        session: SessionId,
        new: &[NewMessage],
    ) -> Result<Vec<MessageId>, StoreError> {
        #[derive(serde::Deserialize)]
        struct Appended {
            ids: Vec<MessageId>,
        }
        let response = self
            .authed(
                self.http
                    .post(self.url(&format!("sessions/{session}/messages"))),
            )
            .json(new)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::CREATED => Ok(response.json::<Appended>().await.map_err(transport)?.ids),
            _ => Err(fail(response).await),
        }
    }

    /// The stream, oldest first; the cursor reads strictly forward.
    pub async fn message_list(
        &self,
        session: SessionId,
        page: &Pagination<MessageId>,
    ) -> Result<Page<Message>, StoreError> {
        self.list(&format!("sessions/{session}/messages"), &(), page)
            .await
    }

    // Signals

    /// Record an observation (born `proposed`).
    pub async fn signal_add(&self, new: &NewSignal) -> Result<SignalId, StoreError> {
        self.create("signals", new).await
    }

    pub async fn signal_get(&self, id: SignalId) -> Result<Option<Signal>, StoreError> {
        self.fetch(&format!("signals/{id}")).await
    }

    pub async fn signal_list(
        &self,
        filter: &SignalFilter,
        page: &Pagination<SignalId>,
    ) -> Result<Page<Signal>, StoreError> {
        self.list("signals", filter, page).await
    }

    /// Signals touching one decision on either end
    /// (`/decisions/{id}/signals`). The filter's `decision`/`project`
    /// must stay unset — the path binds them.
    pub async fn decision_signals(
        &self,
        decision: DecisionId,
        filter: &SignalFilter,
        page: &Pagination<SignalId>,
    ) -> Result<Page<Signal>, StoreError> {
        self.list(&format!("decisions/{decision}/signals"), filter, page)
            .await
    }

    /// Resolve a signal — `Confirmed` or `Dismissed` — stamping who
    /// judged it.
    pub async fn signal_resolve(
        &self,
        id: SignalId,
        status: SignalStatus,
        by: &Author,
    ) -> Result<(), StoreError> {
        #[derive(Serialize)]
        struct Resolve<'a> {
            status: SignalStatus,
            by: &'a Author,
        }
        self.apply(&format!("signals/{id}"), &Resolve { status, by })
            .await
    }

    /// The server's version (open — from the health probe). Distributed
    /// clients compare it against their own build to surface skew.
    pub async fn version(&self) -> Result<String, StoreError> {
        #[derive(serde::Deserialize)]
        struct Health {
            version: String,
        }
        let response = self
            .http
            .get(self.url("healthz"))
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::OK => Ok(response.json::<Health>().await.map_err(transport)?.version),
            _ => Err(fail(response).await),
        }
    }

    /// Auth capabilities (open — the login screen calls this before any
    /// credential exists).
    pub async fn auth_info(&self) -> Result<AuthInfo, StoreError> {
        let response = self
            .http
            .get(self.url("auth"))
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::OK => Ok(response.json().await.map_err(transport)?),
            _ => Err(fail(response).await),
        }
    }

    // Session (the browser's credential exchange)

    /// Exchange a bearer token for the `HttpOnly` session cookie. Browser
    /// use: under wasm the cookie rides fetch ambiently from then on; a
    /// native caller would need a cookie store to retain it (native
    /// callers hold the token itself instead — [`Client::with_token`]).
    pub async fn session_login(&self, token: &str) -> Result<(), StoreError> {
        let response = self
            .http
            .post(self.url("session"))
            .json(&Login { token })
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            _ => Err(fail(response).await),
        }
    }

    /// Clear the session cookie (logout).
    pub async fn session_logout(&self) -> Result<(), StoreError> {
        let response = self
            .http
            .delete(self.url("session"))
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            _ => Err(fail(response).await),
        }
    }

    // Tokens (always the caller's own)

    /// Mint a bearer token; the response carries the secret — the only
    /// time it is ever shown.
    pub async fn token_add(&self, new: &NewToken) -> Result<Minted, StoreError> {
        let response = self
            .authed(self.http.post(self.url("tokens")))
            .json(new)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::CREATED => Ok(response.json().await.map_err(transport)?),
            _ => Err(fail(response).await),
        }
    }

    pub async fn token_list(&self, page: &Pagination<TokenId>) -> Result<Page<Token>, StoreError> {
        self.list("tokens", &(), page).await
    }

    /// Revoke one of the caller's tokens — the credential dies with it.
    pub async fn token_revoke(&self, id: TokenId) -> Result<(), StoreError> {
        let response = self
            .authed(self.http.delete(self.url(&format!("tokens/{id}"))))
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            _ => Err(fail(response).await),
        }
    }

    // Users + agents

    /// The authenticated caller's identity (`/users/me`).
    pub async fn me(&self) -> Result<User, StoreError> {
        self.fetch("users/me").await?.ok_or(StoreError::NotFound)
    }

    pub async fn user_list(&self, page: &Pagination<UserId>) -> Result<Page<User>, StoreError> {
        self.list("users", &(), page).await
    }

    pub async fn agent_list(&self, page: &Pagination<AgentId>) -> Result<Page<Agent>, StoreError> {
        self.list("agents", &(), page).await
    }

    // Plumbing — one helper per HTTP verb shape.

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1/{path}", self.base.as_str().trim_end_matches('/'))
    }

    /// POST a creation; the server answers `201 {"id"}`.
    async fn create<Id: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
    ) -> Result<Id, StoreError> {
        let response = self
            .authed(self.http.post(self.url(path)))
            .json(body)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::CREATED => Ok(response.json::<Created<Id>>().await.map_err(transport)?.id),
            _ => Err(fail(response).await),
        }
    }

    /// GET one resource; `404` is `None`, matching the storage seam.
    async fn fetch<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, StoreError> {
        let response = self
            .authed(self.http.get(self.url(path)))
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::OK => Ok(Some(response.json().await.map_err(transport)?)),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(fail(response).await),
        }
    }

    /// GET a list: filter + pagination ride the query string.
    async fn list<T: DeserializeOwned>(
        &self,
        path: &str,
        filter: &(impl Serialize + ?Sized),
        page: &(impl Serialize + ?Sized),
    ) -> Result<Page<T>, StoreError> {
        let response = self
            .authed(self.http.get(self.url(path)))
            .query(filter)
            .query(page)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::OK => Ok(response.json().await.map_err(transport)?),
            _ => Err(fail(response).await),
        }
    }

    /// PATCH an edit batch; the server answers `204`.
    async fn apply(&self, path: &str, edits: &(impl Serialize + ?Sized)) -> Result<(), StoreError> {
        let response = self
            .authed(self.http.patch(self.url(path)))
            .json(edits)
            .send()
            .await
            .map_err(transport)?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            _ => Err(fail(response).await),
        }
    }
}

/// Map an error response back onto the domain error via the envelope code.
async fn fail(response: Response) -> StoreError {
    let status = response.status();
    match response.json::<Envelope>().await {
        Ok(e) => match e.error.code.as_str() {
            "not_found" => StoreError::NotFound,
            "invalid" => StoreError::Invalid(e.error.message),
            "conflict" => StoreError::Conflict(e.error.message),
            "unauthorized" => StoreError::Unauthorized,
            "unavailable" => StoreError::Unavailable(e.error.message),
            _ => StoreError::Backend(e.error.message),
        },
        Err(_) => StoreError::Backend(format!("unexpected response: {status}")),
    }
}

/// Transport and decode failures — the server never answered (or answered
/// gibberish), so the backend reads as unavailable.
fn transport(e: reqwest::Error) -> StoreError {
    if e.is_decode() {
        StoreError::Backend(format!("malformed response: {e}"))
    } else {
        StoreError::Unavailable(e.to_string())
    }
}
