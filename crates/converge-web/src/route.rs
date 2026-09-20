//! Minimal hash-based router. URLs like `#/`, `#/decision/<id>`, `#/signals`
//! give deep-links and browser back/forward without any server config.

use leptos::prelude::*;

/// Capture this callback before starting async work. It updates the URL and
/// reactive route together, without relying on the later hashchange task.
#[derive(Clone, Copy)]
pub(crate) struct Navigation(pub Callback<Route>);

pub(crate) fn navigation() -> Callback<Route> {
    expect_context::<Navigation>().0
}

#[derive(Clone, PartialEq)]
pub enum Route {
    Dashboard,
    Decision(String),
    Signals,
    SignalDetail(String),
    /// A decision's anchored source: `(decision_id, source_index)`.
    Source(String, usize),
    Project(String),
    /// The active group's own settings: name, description, members, danger.
    GroupSettings,
    /// One project's settings, by project id.
    ProjectSettings(String),
    Search,
    Expert,
    Settings,
    /// Device pairing (RFC 8628 approval): the code may ride the URL
    /// (`#/pair/XXXX-XXXX` from `verification_uri_complete`) or be typed.
    Pair(Option<String>),
    /// A hash path no stock screen owns — offered to the composed
    /// extensions (see `ext`); unresolved, it renders as the dashboard
    /// (the old unknown-hash behavior).
    Ext(String),
}

impl Route {
    pub(crate) fn project_target(&self) -> Option<&str> {
        match self {
            Self::Project(id) | Self::ProjectSettings(id) => Some(id),
            _ => None,
        }
    }

    /// A removed target must not leave editable controls backed by a missing
    /// object. Other routes remain where the user deliberately navigated.
    pub(crate) fn valid_for(self, dataset: &crate::data::Dataset, group: usize) -> Self {
        if self
            .project_target()
            .is_some_and(|id| !dataset.projects.iter().any(|p| p.id == id))
            || (self == Self::GroupSettings && dataset.groups.get(group).is_none())
        {
            Self::Dashboard
        } else {
            self
        }
    }

    pub fn from_hash(hash: &str) -> Route {
        let h = hash.trim_start_matches('#').trim_start_matches('/');
        let mut parts = h.split('/');
        match parts.next().unwrap_or("") {
            "decision" => Route::Decision(parts.next().unwrap_or("").to_string()),
            "signals" => Route::Signals,
            "signal" => Route::SignalDetail(parts.next().unwrap_or("").to_string()),
            "source" => {
                let id = parts.next().unwrap_or("").to_string();
                let idx = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                Route::Source(id, idx)
            }
            "project" => {
                let id = parts.next().unwrap_or("").to_string();
                match parts.next() {
                    Some("settings") => Route::ProjectSettings(id),
                    _ => Route::Project(id),
                }
            }
            // `#/group/settings`; a bare `#/group` is the dashboard, which is
            // the group's own screen.
            "group" => match parts.next() {
                Some("settings") => Route::GroupSettings,
                _ => Route::Dashboard,
            },
            "search" => Route::Search,
            "expert" => Route::Expert,
            "settings" => Route::Settings,
            "pair" => Route::Pair(parts.next().filter(|c| !c.is_empty()).map(str::to_string)),
            "" => Route::Dashboard,
            _ => Route::Ext(h.to_string()),
        }
    }

    pub fn to_hash(&self) -> String {
        match self {
            Route::Dashboard => "#/".into(),
            Route::Decision(id) => format!("#/decision/{id}"),
            Route::Signals => "#/signals".into(),
            Route::SignalDetail(id) => format!("#/signal/{id}"),
            Route::Source(id, idx) => format!("#/source/{id}/{idx}"),
            Route::Project(id) => format!("#/project/{id}"),
            Route::GroupSettings => "#/group/settings".into(),
            Route::ProjectSettings(id) => format!("#/project/{id}/settings"),
            Route::Search => "#/search".into(),
            Route::Expert => "#/expert".into(),
            Route::Settings => "#/settings".into(),
            Route::Pair(None) => "#/pair".into(),
            Route::Pair(Some(code)) => format!("#/pair/{code}"),
            Route::Ext(path) => format!("#/{path}"),
        }
    }

    pub fn crumb(&self) -> String {
        match self {
            Route::Dashboard => "Overview".into(),
            Route::Decision(_) => "Decision".into(),
            Route::Signals => "Signals".into(),
            Route::SignalDetail(_) => "Signal".into(),
            Route::Source(_, _) => "Source".into(),
            Route::Project(id) => id.clone(),
            Route::GroupSettings => "Settings".into(),
            Route::ProjectSettings(_) => "Settings".into(),
            Route::Search => "Search".into(),
            Route::Expert => "Expert model".into(),
            Route::Settings => "Settings".into(),
            Route::Pair(_) => "Pair device".into(),
            // The owning extension's crumb is looked up where the
            // composed set is in scope (TopBar).
            Route::Ext(_) => String::new(),
        }
    }
}

/// The route encoded in the current URL hash.
pub fn current_route() -> Route {
    Route::from_hash(&window().location().hash().unwrap_or_default())
}

/// Push a route into the URL hash (fires `hashchange`).
pub fn navigate(route: &Route) {
    let _ = window().location().set_hash(&route.to_hash());
}
