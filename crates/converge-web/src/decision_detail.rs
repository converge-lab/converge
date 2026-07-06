//! The DecisionDetail view — the richest screen: the decision, its rationale,
//! rejected alternatives, anchored sources, supersession history, cross-project
//! references, and the right-rail provenance + expert payoff. Rendered from the
//! shared dataset, keyed by decision id.

use crate::data;
use crate::route::Route;
use converge_ui::Markdown;
use converge_ui::atoms::{Avatar, Badge, Glyph, SectionLabel};
use converge_ui::domain::{Alternative, ChainNode, CrossRef, Source, Tone};
use converge_ui::molecules::{
    AlternativeCard, CrossRefItem, SignalCard, SignalView, SourceRow, TimelineItem,
};
use leptos::prelude::*;

#[component]
pub fn DecisionDetail(go: Callback<Route>, id: String) -> impl IntoView {
    let Some(d) = data::by_id(&id) else {
        return view! {
            <div class="cv-back" on:click=move |_| go.run(Route::Dashboard)>
                "← Back to dashboard"
            </div>
        }
        .into_any();
    };

    let project = d.project_id.clone();
    let status = d.status;
    let title = d.title.to_string();
    let summary = d.summary.to_string();
    let description = d.description.to_string();
    let has_desc = !description.is_empty();

    // Context is markdown prose; reasons were folded into it at seed
    // conversion, so there is no fallback here anymore.
    let context_md: Option<String> = d.context.clone();
    let consequences: Option<String> = d.consequences.clone();

    let alternatives: Vec<Alternative> = d
        .alts
        .iter()
        .map(|a| Alternative {
            option: a.option.clone(),
            why_rejected: a.why_rejected.clone(),
        })
        .collect();

    // Sources, each deep-linking to the source viewer at its index.
    let sources: Vec<(usize, Source)> = d
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| (i, data::to_source(s)))
        .collect();

    // Supersession chain, newest first; the current node is highlighted.
    let chain_decs = data::chain_of(&id);
    let chain_len = chain_decs.len();
    let mut chain: Vec<(String, ChainNode)> = chain_decs
        .iter()
        .map(|x| {
            (
                x.id.clone(),
                ChainNode {
                    title: x.title.clone(),
                    project: x.project_id.clone(),
                    date: crate::when::when(&x.captured_at),
                    status: x.status,
                    current: x.id == d.id,
                },
            )
        })
        .collect();
    chain.reverse(); // newest first
    let has_chain = chain_len > 1;

    // Cross-project references (outgoing `related_to` edges).
    let related: Vec<(String, CrossRef)> = d
        .related_to
        .iter()
        .filter_map(|x| {
            data::by_id(&x.id).map(|t| {
                (
                    x.id.clone(),
                    CrossRef {
                        project: t.project_id.clone(),
                        title: t.title.clone(),
                        why: x.why.clone(),
                    },
                )
            })
        })
        .collect();
    let has_related = !related.is_empty();

    // Related signals (this decision is the origin or a source of a signal).
    let signals: Vec<(String, _)> = data::signals_for(&id)
        .into_iter()
        .map(|s| (s.id.to_string(), data::to_signal(&s)))
        .collect();
    let has_signals = !signals.is_empty();

    let authors = d.authors.clone();
    let captured = crate::when::when(&d.captured_at);
    let provenance = data::provenance_from(&d);
    let in_expert_context = data::in_agent_context(&d.id, &d.project_id);
    let proj_for_expert = d.project_id.clone();
    let id_for_source = id.clone();

    view! {
        <div>
            <div class="cv-detail">
                // main column
                <div>
                    <div class="cv-row cv-gap-10 cv-mb-14">
                        <span class="cv-mono cv-fs-sm cv-fg-muted">{project}</span>
                        <Badge label=status.label() tone=status.tone() />
                    </div>
                    <h1 class="cv-heading cv-fs-4xl cv-lh-tight cv-mb-22">
                        {title}
                    </h1>

                    {has_signals
                        .then(move || {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-12">
                                        <SectionLabel text="related signals" icon=Glyph::Signal tone=Tone::Signal />
                                    </div>
                                    <div class="cv-stack8">
                                        {signals
                                            .into_iter()
                                            .map(move |(sid, s)| {
                                                view! {
                                                    <SignalCard
                                                        signal=s
                                                        view=SignalView::Related
                                                        on_open=Callback::new(move |_| {
                                                            go.run(Route::SignalDetail(sid.clone()))
                                                        })
                                                    />
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                </div>
                            }
                        })}

                    <div class="cv-detail__section">
                        <div class="cv-mb-12">
                            <SectionLabel text="decision" tone=Tone::Primary bar=true />
                        </div>
                        <div class="cv-detail__decision">
                            <div class="cv-fs-xl cv-fw-medium cv-lh-relaxed">
                                {summary}
                            </div>
                            {has_desc
                                .then(move || {
                                    view! {
                                        <div class="cv-mt-14 cv-pt-16 cv-border-top">
                                            <Markdown source=description />
                                        </div>
                                    }
                                })}
                        </div>
                    </div>

                    {context_md
                        .map(|c| {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-11"><SectionLabel text="context" icon=Glyph::Dashboard /></div>
                                    <div class="cv-detail__prose"><Markdown source=c /></div>
                                </div>
                            }
                        })}

                    {(!alternatives.is_empty())
                        .then(move || {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-12">
                                        <SectionLabel text="alternatives considered" icon=Glyph::Alternative />
                                    </div>
                                    <div class="cv-stack8">
                                        {alternatives
                                            .into_iter()
                                            .map(|a| view! { <AlternativeCard alternative=a /> })
                                            .collect_view()}
                                    </div>
                                </div>
                            }
                        })}

                    {consequences
                        .map(|c| {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-11"><SectionLabel text="consequences" icon=Glyph::Consequence /></div>
                                    <div class="cv-detail__prose"><Markdown source=c /></div>
                                </div>
                            }
                        })}

                    <div class="cv-detail__section">
                        <div class="cv-mb-12">
                            <SectionLabel text="sources · anchored evidence" />
                        </div>
                        <div class="cv-stack8">
                            {sources
                                .into_iter()
                                .map(move |(i, s)| {
                                    let did = id_for_source.clone();
                                    view! {
                                        <SourceRow
                                            source=s
                                            on_view=Callback::new(move |_| {
                                                go.run(Route::Source(did.clone(), i))
                                            })
                                        />
                                    }
                                })
                                .collect_view()}
                        </div>
                    </div>

                    {has_chain
                        .then(move || {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-14">
                                        <SectionLabel text="supersession history" icon=Glyph::Supersede />
                                    </div>
                                    <div>
                                        {chain
                                            .into_iter()
                                            .enumerate()
                                            .map(move |(i, (cid, node))| {
                                                view! {
                                                    <TimelineItem
                                                        node=node
                                                        last=i + 1 == chain_len
                                                        on_open=Callback::new(move |_| go.run(Route::Decision(cid.clone())))
                                                    />
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                </div>
                            }
                        })}

                    {has_related
                        .then(move || {
                            view! {
                                <div class="cv-detail__section">
                                    <div class="cv-mb-10">
                                        <SectionLabel text="references in other projects" />
                                    </div>
                                    <div class="cv-stack8">
                                        {related
                                            .into_iter()
                                            .map(move |(rid, r)| {
                                                view! {
                                                    <CrossRefItem
                                                        cross_ref=r
                                                        on_open=Callback::new(move |_| go.run(Route::Decision(rid.clone())))
                                                    />
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                </div>
                            }
                        })}
                </div>

                // right rail
                <div class="cv-rail">
                    <div class="cv-rail__card">
                        <div class="cv-mb-12"><SectionLabel text="captured by" /></div>
                        <div class="cv-col cv-gap-9 cv-mb-14">
                            {authors
                                .into_iter()
                                .map(|a| {
                                    let color = a.color();
                                    let name = a.name.clone();
                                    view! {
                                        <div class="cv-row cv-gap-10">
                                            <Avatar initial=a.initial color=color size=26 />
                                            <div class="cv-minw-0">
                                                <div class="cv-fs-md cv-fw-medium">{name}</div>
                                                <div class="cv-fs-xs cv-fg-faint">
                                                    "contributor"
                                                </div>
                                            </div>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </div>
                        <div class="cv-border-top cv-pt-12 cv-col cv-gap-7">
                            <div class="cv-rail__meta">
                                <span class="cv-fg-faint">"captured"</span>
                                <span class="cv-fg-secondary">{captured}</span>
                            </div>
                            <div class="cv-rail__meta">
                                <span class="cv-fg-faint">"from"</span>
                                <span class="cv-mono cv-fg-secondary cv-fs-xs">
                                    {provenance}
                                </span>
                            </div>
                        </div>
                    </div>

                    {in_expert_context
                        .then(move || {
                            view! {
                                <div class="cv-known cv-hide-sm" on:click=move |_| go.run(Route::Expert)>
                                    <div class="cv-known__head">
                                        <span class="cv-known__dot"></span>
                                        <span class="cv-known__label">"Known to the expert model"</span>
                                    </div>
                                    <div class="cv-known__text">
                                        "The expert model holds this decision and forwards it to local agents working in "
                                        <span class="cv-mono">{proj_for_expert}</span>
                                        " when it's relevant."
                                    </div>
                                    <div class="cv-known__cta">"Ask the expert →"</div>
                                </div>
                            }
                        })}
                </div>
            </div>
        </div>
    }
    .into_any()
}
