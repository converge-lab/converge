//! The Dashboard view — the group's "recently captured" feed plus the
//! cross-project signals panel. Composed entirely from converge-ui, driven by
//! the shared dataset.

use crate::data;
use crate::route::Route;
use converge_ui::atoms::SectionLabel;
use converge_ui::molecules::{DecisionCard, SignalCard, SignalView};
use leptos::prelude::*;

#[component]
pub fn Dashboard(go: Callback<Route>) -> impl IntoView {
    view! {
        <div class="cv-dash">
            <div class="cv-dash__head">
                <h1 class="cv-heading cv-fs-4xl cv-mb-6">
                    {data::group_name()}
                </h1>
                <p class="cv-fg-muted cv-fs-lg">
                    {data::group_tagline()}" The " <em>"why"</em>
                    " behind the code — captured, anchored, and verifiable."
                </p>
            </div>

            <div class="cv-dash__grid">
                <section>
                    <div class="cv-row cv-gap-8 cv-mb-14">
                        <SectionLabel text="recently captured" />
                        <span class="cv-livedot"></span>
                    </div>
                    <div class="cv-feed">
                        {data::feed()
                            .into_iter()
                            .map(move |d| {
                                let id = d.id.to_string();
                                view! {
                                    <DecisionCard
                                        decision=data::to_card(&d)
                                        on_open=Callback::new(move |_| {
                                            go.run(Route::Decision(id.clone()))
                                        })
                                    />
                                }
                            })
                            .collect_view()}
                    </div>
                </section>

                <div class="cv-dash__aside">
                    <section>
                        <SectionLabel text="cross-project signals" />
                        <div class="cv-stack8 cv-mt-14">
                            {data::group_signals()
                                .into_iter()
                                .map(move |s| {
                                    let id = s.id.to_string();
                                    view! {
                                        <SignalCard
                                            signal=data::to_signal(&s)
                                            view=SignalView::Compact
                                            on_open=Callback::new(move |_| {
                                                go.run(Route::SignalDetail(id.clone()))
                                            })
                                        />
                                    }
                                })
                                .collect_view()}
                        </div>
                    </section>
                </div>
            </div>
        </div>
    }
}
