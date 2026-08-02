//! The Search screen — ranked full-text search across the visible
//! memory (`?q=`, websearch syntax), narrowed by project and status.
//! An empty query browses the recent feed instead; results resolve
//! through the loaded dataset, so rows render with full fidelity
//! (authors, project names) without re-conversion.

use crate::data::{self, Dec};
use crate::route::Route;
use converge_ui::atoms::{Glyph, Input, Select};
use converge_ui::molecules::AvatarStack;
use leptos::prelude::*;
use std::rc::Rc;

/// One search pass. The async side stores plain decision *ids* — the
/// spawned future has no reactive context, so dataset resolution waits
/// for render time. A stale reply (the query moved on) is dropped.
fn run_search(
    query: String,
    project: String,
    status: String,
    current: RwSignal<String>,
    results: RwSignal<Option<Vec<String>>>,
) {
    if query.trim().is_empty() {
        results.set(None);
        return;
    }
    #[cfg(feature = "api")]
    {
        use converge_client::{DecisionFilter, DecisionStatus, ProjectId};
        let filter = DecisionFilter {
            project: (project != "all")
                .then(|| project.parse::<ProjectId>().ok())
                .flatten(),
            group: None,
            status: match status.as_str() {
                "accepted" => Some(DecisionStatus::Accepted),
                "draft" => Some(DecisionStatus::Draft),
                "proposed" => Some(DecisionStatus::Proposed),
                "superseded" => Some(DecisionStatus::Superseded),
                "rejected" => Some(DecisionStatus::Rejected),
                _ => None,
            },
        };
        leptos::task::spawn_local(async move {
            let hits = crate::store::client()
                .decision_search(&query, &filter, Some(30))
                .await
                // A term-free query ("-", "or") is a 400 — render it as
                // no results, not an error state.
                .unwrap_or_default();
            if current.get_untracked() != query {
                return; // stale: the user kept typing
            }
            results.set(Some(hits.iter().map(|d| d.id.to_string()).collect()));
        });
    }
    #[cfg(not(feature = "api"))]
    {
        // Offline analog: case-insensitive substring over the dataset.
        let _ = &current;
        let needle = query.to_lowercase();
        let hits = data::search_local(&needle, &project, &status);
        results.set(Some(hits.iter().map(|d| d.id.clone()).collect()));
    }
}

#[component]
pub fn Search(go: Callback<Route>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let query_value: Signal<String> = query.into();
    let project = RwSignal::new("all".to_string());
    let status = RwSignal::new("all".to_string());
    // `None` = no active search — browse the recent feed. Ids, not rows:
    // render-time resolution keeps the async side context-free.
    let results: RwSignal<Option<Vec<String>>> = RwSignal::new(None);

    let refresh = move || {
        run_search(
            query.get_untracked(),
            project.get_untracked(),
            status.get_untracked(),
            query,
            results,
        )
    };

    let mut project_options = vec![("all".to_string(), "All projects".to_string())];
    project_options.extend(
        data::cur_group_projects()
            .into_iter()
            .map(|pid| (pid.clone(), data::proj_name(&pid))),
    );

    let row = move |d: Rc<Dec>| {
        let id = d.id.to_string();
        let authors = d.authors.clone();
        let title = d.title.to_string();
        let proj = data::proj_name(&d.project_id);
        view! {
            <div class="cv-browse" on:click=move |_| go.run(Route::Decision(id.clone()))>
                <AvatarStack authors=authors size=16 max=2 />
                <span class="cv-browse__title">{title}</span>
                <span class="cv-browse__proj cv-mono">{proj}</span>
            </div>
        }
    };

    view! {
        <div class="cv-page">
            <div class="cv-search__hero cv-mb-28">
                <div class="cv-filterlabel cv-mb-14 cv-ls-wider">
                    "Search the group's memory"
                </div>
                <div class="cv-w-full cv-measure">
                    <Input
                        placeholder="Search decisions, rationale, tags…"
                        lead=Glyph::Search
                        value=query_value
                        on_input=Callback::new(move |q: String| {
                            query.set(q);
                            refresh();
                        })
                    />
                </div>
                <div class="cv-row cv-jc-center cv-wrap cv-gap-10 cv-mt-14">
                    <Select
                        options=project_options
                        on_change=Callback::new(move |p: String| {
                            project.set(p);
                            refresh();
                        })
                    />
                    <Select
                        options=vec![
                            ("all".to_string(), "All statuses".to_string()),
                            ("accepted".to_string(), "Accepted".to_string()),
                            ("draft".to_string(), "Draft".to_string()),
                            ("proposed".to_string(), "Proposed".to_string()),
                            ("superseded".to_string(), "Superseded".to_string()),
                            ("rejected".to_string(), "Rejected".to_string()),
                        ]
                        on_change=Callback::new(move |s: String| {
                            status.set(s);
                            refresh();
                        })
                    />
                </div>
            </div>

            {move || match results.get() {
                None => {
                    view! {
                        <h1 class="cv-heading cv-fs-3xl cv-mb-4">
                            "Search across every group"
                        </h1>
                        <p class="cv-fs-lg cv-fg-muted cv-mb-24">
                            "Ranked full-text over titles, summaries and rationale — bare words AND, `or` alternates, `-` excludes, quoted phrases match exactly."
                        </p>
                        <div class="cv-filterlabel cv-mb-12 cv-ls-wide">
                            "Recently captured"
                        </div>
                        <div class="cv-stack8">
                            {data::feed().into_iter().map(row).collect_view()}
                        </div>
                    }
                        .into_any()
                }
                Some(ids) => {
                    let hits: Vec<Rc<Dec>> =
                        ids.iter().filter_map(|id| data::by_id(id)).collect();
                    if hits.is_empty() {
                        return view! {
                            <div class="cv-filterlabel cv-mb-12 cv-ls-wide">"No matches"</div>
                            <p class="cv-fs-lg cv-fg-muted">
                                "Nothing matches that query — try fewer or different words."
                            </p>
                        }
                        .into_any();
                    }
                    let n = hits.len();
                    view! {
                        <div class="cv-filterlabel cv-mb-12 cv-ls-wide">
                            {format!(
                                "{n} {} · best match first",
                                if n == 1 { "result" } else { "results" },
                            )}
                        </div>
                        <div class="cv-stack8">
                            {hits.into_iter().map(row).collect_view()}
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}
