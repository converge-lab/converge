//! Minimal hash-based router. URLs like `#/`, `#/decision/<id>`, `#/signals`
//! give deep-links and browser back/forward without any server config.

use leptos::prelude::*;

#[derive(Clone, PartialEq)]
pub enum Route {
    Dashboard,
    Decision(String),
    Signals,
    SignalDetail(String),
    /// A decision's anchored source: `(decision_id, source_index)`.
    Source(String, usize),
    Project(String),
    Search,
    Expert,
    Settings,
    /// Device pairing (RFC 8628 approval): the code may ride the URL
    /// (`#/pair/XXXX-XXXX` from `verification_uri_complete`) or be typed.
    Pair(Option<String>),
}

impl Route {
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
            "project" => Route::Project(parts.next().unwrap_or("").to_string()),
            "search" => Route::Search,
            "expert" => Route::Expert,
            "settings" => Route::Settings,
            "pair" => Route::Pair(parts.next().filter(|c| !c.is_empty()).map(str::to_string)),
            _ => Route::Dashboard,
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
            Route::Search => "#/search".into(),
            Route::Expert => "#/expert".into(),
            Route::Settings => "#/settings".into(),
            Route::Pair(None) => "#/pair".into(),
            Route::Pair(Some(code)) => format!("#/pair/{code}"),
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
            Route::Search => "Search".into(),
            Route::Expert => "Expert model".into(),
            Route::Settings => "Settings".into(),
            Route::Pair(_) => "Pair device".into(),
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
