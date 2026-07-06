//! The Search screen — search across the group's memory. The query and filters
//! are presentational for now; below them is the browse list of recent
//! decisions, each a light row linking to its decision.

use crate::data;
use crate::route::Route;
use converge_ui::atoms::{Glyph, Input, Select};
use converge_ui::molecules::AvatarStack;
use leptos::prelude::*;

#[component]
pub fn Search(go: Callback<Route>) -> impl IntoView {
    view! {
        <div class="cv-page">
            <div class="cv-search__hero cv-mb-28">
                <div class="cv-filterlabel cv-mb-14 cv-ls-wider">
                    "Search the group's memory"
                </div>
                <div class="cv-w-full cv-measure">
                    <Input placeholder="Search decisions, rationale, tags…" lead=Glyph::Search />
                </div>
                <div class="cv-row cv-jc-center cv-wrap cv-gap-10 cv-mt-14">
                    <Select options=vec![
                        ("all".to_string(), "All projects".to_string()),
                        ("api-gateway".to_string(), "api-gateway".to_string()),
                        ("web-app".to_string(), "web-app".to_string()),
                    ] />
                    <Select options=vec![
                        ("all".to_string(), "All statuses".to_string()),
                        ("accepted".to_string(), "Accepted".to_string()),
                        ("superseded".to_string(), "Superseded".to_string()),
                    ] />
                    <Select options=vec![
                        ("all".to_string(), "All authors".to_string()),
                        ("human".to_string(), "People".to_string()),
                        ("agent".to_string(), "Agents".to_string()),
                    ] />
                </div>
            </div>

            <h1 class="cv-heading cv-fs-3xl cv-mb-4">
                "Search across every group"
            </h1>
            <p class="cv-fs-lg cv-fg-muted cv-mb-24">
                "Decisions from all your groups and projects, each with its matched source. Start typing above, or jump to a recent one."
            </p>
            <div class="cv-filterlabel cv-mb-12 cv-ls-wide">
                "Recently captured"
            </div>
            <div class="cv-stack8">
                {data::feed()
                    .into_iter()
                    .map(move |d| {
                        let id = d.id.to_string();
                        let authors = d.authors.clone();
                        let title = d.title.to_string();
                        let proj = d.project_id.clone();
                        view! {
                            <div
                                class="cv-browse"
                                on:click=move |_| go.run(Route::Decision(id.clone()))
                            >
                                <AvatarStack authors=authors size=16 max=2 />
                                <span class="cv-browse__title">{title}</span>
                                <span class="cv-browse__proj cv-mono">{proj}</span>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}
