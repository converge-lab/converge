//! The Dashboard view — the group's "recently captured" feed plus the
//! cross-project signals panel. Composed entirely from converge-ui, driven by
//! the shared dataset.

use crate::command_snippet::{CommandSnippet, install_command};
use crate::data;
use crate::onboard::Onboarding;
use crate::route::Route;
use converge_ui::atoms::{Glyph, SectionLabel};
use converge_ui::molecules::{DecisionCard, MenuItem, OverflowMenu, SignalCard, SignalView};
use leptos::ev;
use leptos::prelude::*;

#[component]
pub fn Dashboard(go: Callback<Route>) -> impl IntoView {
    // With no signals the aside is dropped entirely — the grid collapses
    // to one column so the feed isn't stranded beside a phantom column.
    let solo = data::group_signals().is_empty();

    // The group's "⋯" menu, the twin of the project log's header menu:
    // Escape closes it, outside clicks land on the scrim below. The listener
    // handle is removed by hand — leptos never detaches window listeners on
    // owner disposal, and the dashboard is rebuilt on every navigation, so a
    // dropped handle would leak one global listener per visit.
    let (menu_open, set_menu_open) = signal(false);
    let escape = window_event_listener(ev::keydown, move |evt| {
        if evt.key() == "Escape" {
            set_menu_open.set(false);
        }
    });
    on_cleanup(move || escape.remove());

    view! {
        <div class="cv-dash">
            <div class="cv-dash__head cv-row">
                <div class="cv-grow">
                    <h1 class="cv-heading cv-fs-4xl cv-mb-6">
                        {data::group_name()}
                    </h1>
                    <p class="cv-fg-muted cv-fs-lg">
                        {data::group_tagline()}" The " <em>"why"</em>
                        " behind the code — captured, anchored, and verifiable."
                    </p>
                </div>
                <div class="cv-relative">
                    <div
                        class=move || {
                            if menu_open.get() {
                                "cv-iconbtn cv-iconbtn--open"
                            } else {
                                "cv-iconbtn"
                            }
                        }
                        role="button"
                        tabindex="0"
                        aria-label="Group actions"
                        on:click=move |_| set_menu_open.update(|o| *o = !*o)
                        on:keydown=move |ev| {
                            if ev.key() == "Enter" || ev.key() == " " {
                                ev.prevent_default();
                                set_menu_open.update(|o| *o = !*o);
                            }
                        }
                    >
                        {Glyph::More.glyph()}
                    </div>
                    {move || {
                        menu_open
                            .get()
                            .then(|| {
                                view! {
                                    // Transparent full-screen catcher: an outside click closes the menu.
                                    <div
                                        class="cv-acctmenu__scrim"
                                        on:click=move |_| set_menu_open.set(false)
                                    ></div>
                                    // Anchored top-right below the button; z above the scrim. No
                                    // utility covers this float, so the position is inline (as in
                                    // the project log's menu).
                                    <div style="position:absolute;right:0;top:2.25rem;z-index:61">
                                        <OverflowMenu>
                                            // One entry, the same rule the
                                            // project's "⋯" follows: the menu
                                            // beside a name opens that object's
                                            // settings. Name, description,
                                            // members and the destructive
                                            // actions all live on that screen.
                                            <MenuItem
                                                icon=Glyph::Settings
                                                label="Settings"
                                                on_click=Callback::new(move |_| {
                                                    set_menu_open.set(false);
                                                    go.run(Route::GroupSettings);
                                                })
                                            />
                                        </OverflowMenu>
                                    </div>
                                }
                            })
                    }}
                </div>
            </div>

            // An empty group keeps the header above (its "⋯" is the way
            // into settings — invite/rename/delete matter most before the
            // first project); only the content below flips to the guide.
            {if data::cur_group_projects().is_empty() {
                view! { <Onboarding /> }.into_any()
            } else {
                view! {
            <div class=if solo { "cv-dash__grid cv-dash__grid--solo" } else { "cv-dash__grid" }>
                <section>
                    <div class="cv-row cv-gap-8 cv-mb-14">
                        <SectionLabel text="recently captured" />
                        <span class="cv-livedot"></span>
                    </div>
                    <div class="cv-feed">
                        {
                            let feed = data::feed();
                            if feed.is_empty() {
                                // Projects exist but nothing recorded yet — nudge toward the agent.
                                view! {
                                    <div class="cv-onboard__agent">
                                        <div class="cv-fs-md cv-fg-muted">
                                            "No decisions yet. Connect your agent and record the first one — it lands here, anchored to its source."
                                        </div>
                                        <CommandSnippet command=install_command() />
                                    </div>
                                }
                                    .into_any()
                            } else {
                                feed.into_iter()
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
                                    .collect_view()
                                    .into_any()
                            }
                        }
                    </div>
                </section>

                {
                    let signals = data::group_signals();
                    // The signals panel is hidden entirely when there are none.
                    (!signals.is_empty())
                        .then(|| {
                            view! {
                                <div class="cv-dash__aside">
                                    <section>
                                        <SectionLabel text="cross-project signals" />
                                        <div class="cv-stack8 cv-mt-14">
                                            {signals
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
                            }
                        })
                }
            </div>
                }
                    .into_any()
            }}
        </div>
    }
}
