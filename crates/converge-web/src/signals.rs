//! The Signals screens — the cross-project signals list and a SignalDetail.

use crate::data;
use crate::route::Route;
use converge_ui::atoms::{Badge, Glyph, Icon, SectionLabel};
use converge_ui::domain::Tone;
use converge_ui::molecules::{DecisionMiniRow, LegendItem, SignalCard, SignalView};
use leptos::prelude::*;

#[component]
pub fn Signals(go: Callback<Route>) -> impl IntoView {
    view! {
        <div class="cv-page">
            <div class="cv-row cv-gap-10 cv-mb-6">
                <Icon glyph=Glyph::Signal size=17 color=Tone::Signal.color_var() />
                <h1 class="cv-heading cv-fs-4xl">
                    "Cross-project signals"
                </h1>
            </div>
            <p class="cv-fg-muted cv-fs-lg cv-mb-18 cv-lh-normal">
                "Where a decision in one project affects another. Converge watches the whole group's memory and surfaces these so a change never silently breaks a neighbor."
            </p>

            <div class="cv-row cv-gap-16 cv-wrap cv-mb-22">
                <LegendItem tone=Tone::Danger term="Will break" desc="active conflict, fails if shipped" />
                <LegendItem tone=Tone::Signal term="Coordinate" desc="needs cross-team timing" />
                <LegendItem tone=Tone::Expert term="Watch" desc="standing dependency to monitor" />
            </div>

            <div class="cv-col cv-gap-10">
                {data::group_signals()
                    .into_iter()
                    .map(move |s| {
                        let id = s.id.to_string();
                        let n = s.sources.len() as u32;
                        view! {
                            <SignalCard
                                signal=data::to_signal(&s)
                                view=SignalView::Full
                                source_count=n
                                on_open=Callback::new(move |_| go.run(Route::SignalDetail(id.clone())))
                            />
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}

#[component]
pub fn SignalDetail(go: Callback<Route>, id: String) -> impl IntoView {
    let back = move |_| go.run(Route::Signals);
    let Some(sig) = data::sig_by_id(&id) else {
        return view! { <div class="cv-back" on:click=back>"← All signals"</div> }.into_any();
    };
    let detail = data::to_signal_detail(&sig);
    let risk = detail.risk;
    // Pair each source id with its rendered ref so the row can deep-link.
    let sources: Vec<(String, _)> = sig
        .sources
        .iter()
        .filter_map(|sid| data::by_id(sid).map(|d| (sid.to_string(), data::to_ref(&d))))
        .collect();

    view! {
        <div class="cv-page cv-page--narrow">
            <div class="cv-back" on:click=back>"← All signals"</div>

            <div class="cv-row cv-gap-10 cv-wrap cv-mb-12">
                <span class="cv-mono cv-fs-lg">{detail.from}</span>
                <span class="cv-fg-faint">"→"</span>
                <span class="cv-mono cv-fs-lg">{detail.to}</span>
                <Badge label=risk.label() tone=risk.tone() />
            </div>
            <h1 class="cv-heading cv-fs-4xl cv-lh-tight cv-mb-22">
                {detail.title}
            </h1>

            <div class="cv-mb-24">
                <div class=format!("cv-whathappens cv-whathappens--{}", risk.tone().slug())>
                    <div class="cv-whathappens__head">
                        <span class="cv-whathappens__bang">"!"</span>
                        <span>"What happens"</span>
                    </div>
                    <div class="cv-whathappens__body">{detail.consequence}</div>
                </div>
            </div>

            <div class="cv-mb-28">
                <div class="cv-mb-10"><SectionLabel text="recommended action" /></div>
                <div class="cv-recommend">
                    <span class="cv-recommend__check">{Glyph::Verified.glyph()}</span>
                    <div class="cv-recommend__body">{detail.recommended}</div>
                </div>
            </div>

            <div>
                <div class="cv-mb-12"><SectionLabel text="source decisions" /></div>
                <div class="cv-stack8">
                    {sources
                        .into_iter()
                        .map(move |(sid, d)| {
                            view! {
                                <DecisionMiniRow
                                    decision=d
                                    on_open=Callback::new(move |_| {
                                        go.run(Route::Decision(sid.clone()))
                                    })
                                />
                            }
                        })
                        .collect_view()}
                </div>
            </div>
        </div>
    }
    .into_any()
}
