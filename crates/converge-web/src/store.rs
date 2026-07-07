//! The reactive application store.
//!
//! Everything the app renders lives in one `Store<AppState>` (from
//! `reactive_stores`), handed to every component through Leptos context. The
//! store gives *fine-grained* reactivity: updating `group` never notifies the
//! readers of `dataset`, and vice-versa — so a group switch doesn't invalidate
//! the whole tree.
//!
//! Data enters the store through a [`DataSource`]: [`EmbeddedSource`] parses
//! and assembles the embedded fixture seed (`crate::seed`); the HTTP
//! `ApiSource` on `converge-client` returns in the next slice. Both funnel
//! through `data::build_dataset`, so the two paths cannot drift.

use crate::data::{Dataset, build_dataset};
use crate::seed::{EMBEDDED, Seed, assemble};
use leptos::prelude::*;
use leptos::task::spawn_local;
use reactive_stores::Store;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// The whole reactive state of the app.
#[derive(Clone, Default, Store)]
pub struct AppState {
    /// The loaded dataset. `None` while the source is still resolving. The UI
    /// gates on this: nothing that queries the dataset renders until it's
    /// `Some`, so query functions can assume a loaded dataset.
    pub dataset: Option<Rc<Dataset>>,
    /// Set if loading failed. When present, the app shows an error screen
    /// (or the login screen, when the failure is a missing credential).
    pub error: Option<LoadError>,
    /// Index of the active group within `dataset.groups`.
    pub group: usize,
}

/// `Rc` inside the state makes it `!Send`, so the store is pinned to the single
/// WASM thread via `LocalStorage` — exactly right for a client-side app, and it
/// spares us needless `Send + Sync` bounds.
pub type AppStore = Store<AppState, LocalStorage>;

/// Why a load failed. The embedded source never fails; an HTTP `ApiSource`
/// distinguishes "you need to log in" from everything else.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// The API wants a credential the browser doesn't hold — show login.
    Unauthorized,
    /// Anything else, human-readable.
    Failed(String),
}

/// A pending dataset load. The future is intentionally *not* `Send`: it runs on
/// the single WASM thread (via `spawn_local`) and an HTTP source's `fetch`
/// future is `!Send` anyway.
pub type Loading = Pin<Box<dyn Future<Output = Result<Rc<Dataset>, LoadError>>>>;

/// Where the dataset comes from. Async so an HTTP `ApiSource` slots in behind
/// the same trait without touching the store or any component.
pub trait DataSource {
    fn load(&self) -> Loading;
}

/// The embedded fixture seed, assembled in-process. Resolves immediately;
/// stays useful as the offline / test fixture even after a real API exists.
/// Unused (but still compiled) when the `api` feature is on.
#[cfg_attr(feature = "api", allow(dead_code))]
pub struct EmbeddedSource;

impl DataSource for EmbeddedSource {
    fn load(&self) -> Loading {
        Box::pin(async {
            let seed = Seed::parse(EMBEDDED).expect("embedded seed is malformed");
            // A broken seed is a build-time mistake; validation is a cheap
            // safety net in dev builds and dead weight in release.
            #[cfg(debug_assertions)]
            if let Err(errs) = crate::seed::validate(&seed) {
                panic!("embedded seed invalid:\n{}", errs.join("\n"));
            }
            Ok(Rc::new(build_dataset(assemble(&seed))))
        })
    }
}

/// Build an empty store, publish it to context, and kick off the async load
/// from `source`. Returns the (copyable) handle for the caller to keep. When
/// the load resolves it writes into the store, flipping the UI from its loading
/// state to the app (or to an error screen on failure).
pub fn provide_store<S: DataSource + 'static>(source: S) -> AppStore {
    let store = AppStore::new_local(AppState::default());
    provide_context(store);
    spawn_local(async move {
        match source.load().await {
            Ok(dataset) => store.dataset().set(Some(dataset)),
            Err(err) => store.error().set(Some(err)),
        }
    });
    store
}

/// Provide the store from the default source for this build: the HTTP
/// [`api::ApiSource`] when compiled with the `api` feature, otherwise the
/// [`EmbeddedSource`]. This is the one line to flip between them.
pub fn provide_default_store() -> AppStore {
    #[cfg(feature = "api")]
    {
        provide_store(api::ApiSource::same_origin())
    }
    #[cfg(not(feature = "api"))]
    {
        provide_store(EmbeddedSource)
    }
}

/// The HTTP data source, on the typed `converge-client`. Real resources come
/// from the API; the not-yet-real remainder (signals, unread, extras, expert
/// context — M4 territory) still comes from the embedded seed, so mocked
/// features are visibly seed-scoped in exactly one place.
#[cfg(feature = "api")]
pub use api::client;

#[cfg(feature = "api")]
mod api {
    use super::{DataSource, LoadError, Loading, build_dataset};
    use crate::seed::{self, wire};
    use converge_client::{Client, DecisionFilter, Pagination, ProjectFilter, StoreError};
    use converge_ui::domain::initials;
    use leptos::prelude::window;
    use std::rc::Rc;
    use time::format_description::well_known::Rfc3339;

    pub struct ApiSource {
        client: Client,
    }

    impl ApiSource {
        /// The API lives at the app's own origin: `trunk serve` proxies
        /// `/api` to the local server in dev, and in production the server
        /// binary serves these assets itself. No baked-in URLs.
        pub fn same_origin() -> Self {
            Self { client: client() }
        }
    }

    /// A same-origin client. Session-cookie auth rides the browser's fetch
    /// ambiently — no token is ever held in the app.
    pub fn client() -> Client {
        let origin = window().location().origin().expect("window has an origin");
        let base = origin.parse().expect("origin is a valid URL");
        Client::new(base)
    }

    fn oops(what: &str) -> impl Fn(StoreError) -> LoadError + '_ {
        move |e| match e {
            StoreError::Unauthorized => LoadError::Unauthorized,
            e => LoadError::Failed(format!("{what}: {e}")),
        }
    }

    impl DataSource for ApiSource {
        fn load(&self) -> Loading {
            let client = self.client.clone();
            Box::pin(async move {
                // Unpaginated boot loads: without `limit` the server returns
                // everything — fine at v1 scale, cursor-walking later.
                let me = client.me().await.map_err(oops("load identity"))?;
                let groups = client
                    .group_list(&Pagination::default())
                    .await
                    .map_err(oops("load groups"))?;
                let projects = client
                    .project_list(&ProjectFilter::default(), &Pagination::default())
                    .await
                    .map_err(oops("load projects"))?;
                let users = client
                    .user_list(&Pagination::default())
                    .await
                    .map_err(oops("load users"))?;
                let agents = client
                    .agent_list(&Pagination::default())
                    .await
                    .map_err(oops("load agents"))?;
                let decisions = client
                    .decision_list(&DecisionFilter::default(), &Pagination::default())
                    .await
                    .map_err(oops("load decisions"))?;

                // The read-model wants each decision's one-hop edges; the API
                // serves them as a projection per decision. Sequential is fine
                // at boot-load scale.
                let mut wired = Vec::with_capacity(decisions.items.len());
                for d in &decisions.items {
                    let edges = client
                        .decision_edges(d.id)
                        .await
                        .map_err(oops("load edges"))?
                        .unwrap_or_default();
                    wired.push(decision(d, &edges));
                }

                // M4 residue from the fixture seed. Its ids don't intersect
                // real data, so these features read as empty until their
                // endpoints exist — honest, and contained here.
                let seed = seed::Seed::parse(seed::EMBEDDED).expect("embedded seed is malformed");
                let mock = seed::assemble(&seed);

                let assembled = seed::Assembled {
                    groups: groups
                        .items
                        .iter()
                        .map(|g| group(g, &projects.items))
                        .collect(),
                    projects: projects.items.iter().map(project).collect(),
                    users: users.items.iter().map(user).collect(),
                    agents: agents.items.iter().map(agent).collect(),
                    decisions: wired,
                    me: wire::mock::Me {
                        user_id: me.id.to_string(),
                        initial: initials(&me.name),
                        name: me.name,
                        role: format!("@{}", me.handle),
                        color: "var(--cv-primary)".into(),
                        email: String::new(),
                    },
                    user_colors: mock.user_colors,
                    signals: mock.signals,
                    decision_extras: mock.decision_extras,
                    unread: mock.unread,
                    agent_context: mock.agent_context,
                };
                Ok(Rc::new(build_dataset(assembled)))
            })
        }
    }

    // Typed client responses → the app's read-model shapes. This is the one
    // place the real wire meets the fixture format, converted in code the
    // compiler checks against `converge-client`.

    fn rfc3339(t: time::OffsetDateTime) -> String {
        t.format(&Rfc3339).expect("timestamps format as RFC3339")
    }

    fn group(g: &converge_client::Group, projects: &[converge_client::Project]) -> wire::Group {
        wire::Group {
            id: g.id.to_string(),
            name: g.name.clone(),
            description: g.description.clone(),
            kind: match g.kind {
                converge_client::GroupKind::Shared => seed::GroupKind::Shared,
                converge_client::GroupKind::Personal => seed::GroupKind::Personal,
            },
            created_at: rfc3339(g.created_at),
            // The D3 membership read-model, derived client-side.
            project_ids: projects
                .iter()
                .filter(|p| p.group_id == g.id)
                .map(|p| p.id.to_string())
                .collect(),
        }
    }

    fn project(p: &converge_client::Project) -> wire::Project {
        wire::Project {
            id: p.id.to_string(),
            group_id: p.group_id.to_string(),
            name: p.name.clone(),
            description: p.description.clone(),
            created_at: rfc3339(p.created_at),
        }
    }

    fn user(u: &converge_client::User) -> wire::User {
        wire::User {
            id: u.id.to_string(),
            handle: u.handle.clone(),
            name: u.name.clone(),
        }
    }

    fn agent(a: &converge_client::Agent) -> wire::Agent {
        wire::Agent {
            id: a.id.to_string(),
            kind: match a.kind {
                converge_client::AgentKind::Model => seed::enums::AgentKind::Model,
                converge_client::AgentKind::Tool => seed::enums::AgentKind::Tool,
            },
            name: a.name.clone(),
        }
    }

    fn author(a: &converge_client::Author) -> wire::AuthorRef {
        use converge_client::Author as A;
        match a {
            A::User(u) => wire::AuthorRef {
                user_id: Some(u.to_string()),
                agent_id: None,
            },
            A::Agent(g) => wire::AuthorRef {
                user_id: None,
                agent_id: Some(g.to_string()),
            },
            A::UserViaAgent { user, agent } => wire::AuthorRef {
                user_id: Some(user.to_string()),
                agent_id: Some(agent.to_string()),
            },
        }
    }

    fn status(s: converge_client::DecisionStatus) -> seed::Status {
        use converge_client::DecisionStatus as W;
        match s {
            W::Accepted => seed::Status::Accepted,
            W::Draft => seed::Status::Draft,
            W::Proposed => seed::Status::Proposed,
            W::Superseded => seed::Status::Superseded,
            W::Rejected => seed::Status::Rejected,
        }
    }

    fn related(r: &converge_client::Related) -> wire::RelatedRef {
        wire::RelatedRef {
            id: r.id.to_string(),
            why: r.why.clone(),
        }
    }

    fn decision(d: &converge_client::Decision, e: &converge_client::Edges) -> wire::Decision {
        wire::Decision {
            id: d.id.to_string(),
            project_id: d.project_id.to_string(),
            status: status(d.status),
            title: d.title.clone(),
            summary: d.summary.clone(),
            context: d.context.clone(),
            consequences: d.consequences.clone(),
            alternatives: d
                .alternatives
                .iter()
                .map(|a| wire::Alternative {
                    option: a.option.clone(),
                    why_rejected: a.why_rejected.clone(),
                })
                .collect(),
            authors: d.authors.iter().map(author).collect(),
            supersedes: e.supersedes.iter().map(|i| i.to_string()).collect(),
            superseded_by: e.superseded_by.iter().map(|i| i.to_string()).collect(),
            related_to: e.related_to.iter().map(related).collect(),
            related_by: e.related_by.iter().map(related).collect(),
            captured_at: rfc3339(d.captured_at),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The author pair mapping preserves all three enum states.
        #[test]
        fn author_states_map() {
            use converge_client::{AgentId, Author as A, UserId};
            let (u, g) = (UserId::new(), AgentId::new());
            assert_eq!(author(&A::User(u)).user_id, Some(u.to_string()));
            assert_eq!(author(&A::User(u)).agent_id, None);
            assert_eq!(author(&A::Agent(g)).agent_id, Some(g.to_string()));
            let via = author(&A::UserViaAgent { user: u, agent: g });
            assert_eq!(via.user_id, Some(u.to_string()));
            assert_eq!(via.agent_id, Some(g.to_string()));
        }
    }
}

/// The store for the current reactive owner.
///
/// Panics if [`provide_store`] wasn't called at the root — a programming error,
/// not a runtime condition.
pub fn use_store() -> AppStore {
    expect_context::<AppStore>()
}
