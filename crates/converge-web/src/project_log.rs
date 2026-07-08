//! The ProjectLog — a single project's decision log as a bordered table, with
//! reactive status / author / tag filters above it.

use crate::data;
use crate::modals::{ModalKind, open};
use crate::route::Route;
use converge_ui::atoms::{Glyph, Select};
use converge_ui::domain::{DecisionRef, Status};
use converge_ui::molecules::{DecisionLogRow, MenuItem, OverflowMenu, TagFilterMenu};
use leptos::ev;
use leptos::prelude::*;

/// Everything a log row needs to render *and* be filtered — precomputed from the
/// `Rc<Dec>` snapshot into owned, `Send` data so the reactive row/count closures
/// (which Leptos requires to be `Send`) don't capture the `!Send` `Rc`s.
#[derive(Clone)]
struct RowData {
    id: String,
    date: String,
    unread: bool,
    status: Status,
    authors: Vec<String>,
    tags: Vec<String>,
    dref: DecisionRef,
}

#[component]
pub fn ProjectLog(go: Callback<Route>, pid: String) -> impl IntoView {
    let decs = data::project_decisions(&pid);

    // Filter option sets, derived from the project's decisions. Author options
    // are names — the resolved `domain::Author` values carry them.
    let mut authors: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for d in &decs {
        for a in &d.authors {
            if !authors.iter().any(|x| x == &a.name) {
                authors.push(a.name.clone());
            }
        }
        for t in &d.tags {
            if !tags.iter().any(|x| x == t) {
                tags.push(t.clone());
            }
        }
    }
    authors.sort();

    let (fstatus, set_fstatus) = signal(String::from("all"));
    let (fauthor, set_fauthor) = signal(String::from("all"));
    let (ftags, set_ftags) = signal::<Vec<String>>(Vec::new());

    // The header "⋯" overflow menu (Edit only this iteration). Escape closes it;
    // outside clicks are caught by a transparent scrim below.
    let (menu_open, set_menu_open) = signal(false);
    let menu_pid = pid.clone();
    window_event_listener(ev::keydown, move |evt| {
        if evt.key() == "Escape" {
            set_menu_open.set(false);
        }
    });

    let heading = data::proj_name(&pid);
    let desc = data::proj_desc(&pid);

    let mut status_opts = vec![("all".to_string(), "All statuses".to_string())];
    for (v, l) in [
        ("accepted", "Accepted"),
        ("superseded", "Superseded"),
        ("rejected", "Rejected"),
        ("draft", "Draft"),
    ] {
        status_opts.push((v.to_string(), l.to_string()));
    }
    let mut author_opts = vec![("all".to_string(), "All authors".to_string())];
    for a in &authors {
        author_opts.push((a.clone(), a.clone()));
    }

    // Precompute owned, `Send` row data once from the snapshot.
    let row_data: Vec<RowData> = decs
        .iter()
        .map(|d| RowData {
            id: d.id.clone(),
            date: crate::when::when(&d.captured_at),
            unread: data::is_unread(&d.id),
            status: d.status,
            authors: d.authors.iter().map(|a| a.name.clone()).collect(),
            tags: d.tags.clone(),
            dref: data::to_ref(d),
        })
        .collect();

    // One predicate, read by both the row list and the count, so they can't drift.
    let keep = move |r: &RowData| {
        let fs = fstatus.get();
        let fa = fauthor.get();
        let ft = ftags.get();
        (fs == "all" || r.status.label() == fs)
            && (fa == "all" || r.authors.contains(&fa))
            && (ft.is_empty() || r.tags.iter().any(|t| ft.iter().any(|x| x == t)))
    };

    let rows_data = row_data.clone();
    let rows = move || {
        rows_data
            .iter()
            .filter(|r| keep(r))
            .map(|r| {
                let id = r.id.clone();
                view! {
                    <DecisionLogRow
                        decision=r.dref.clone()
                        date=r.date.clone()
                        unread=r.unread
                        on_open=Callback::new(move |_| go.run(Route::Decision(id.clone())))
                    />
                }
            })
            .collect_view()
    };
    let count = move || row_data.iter().filter(|r| keep(r)).count();

    view! {
        <div class="cv-page">
            <div class="cv-row cv-mb-22">
                <div class="cv-grow">
                    <h1 class="cv-mono cv-fs-3xl cv-fw-medium cv-mb-4">
                        {heading}
                    </h1>
                    <p class="cv-fg-muted cv-fs-lg">{desc}</p>
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
                                let pid = menu_pid.clone();
                                view! {
                                    // Transparent full-screen catcher: an outside click closes the menu.
                                    <div
                                        class="cv-acctmenu__scrim"
                                        on:click=move |_| set_menu_open.set(false)
                                    ></div>
                                    // Anchored top-right below the button; z above the scrim. No
                                    // utility covers this float, so the position is inline (as in the
                                    // prototype).
                                    <div style="position:absolute;right:0;top:2.25rem;z-index:61">
                                        <OverflowMenu>
                                            <MenuItem
                                                icon=Glyph::Edit
                                                label="Edit project…"
                                                on_click=Callback::new(move |_| {
                                                    set_menu_open.set(false);
                                                    open(ModalKind::EditProject(pid.clone()));
                                                })
                                            />
                                        </OverflowMenu>
                                    </div>
                                }
                            })
                    }}
                </div>
            </div>

            <div class="cv-row cv-wrap cv-gap-18 cv-mb-18">
                <div class="cv-row cv-gap-6">
                    <span class="cv-filterlabel">"Status"</span>
                    <Select
                        options=status_opts
                        on_change=Callback::new(move |v| set_fstatus.set(v))
                    />
                </div>
                <div class="cv-row cv-gap-6">
                    <span class="cv-filterlabel">"Author"</span>
                    <Select
                        options=author_opts
                        on_change=Callback::new(move |v| set_fauthor.set(v))
                    />
                </div>
                <div class="cv-row cv-gap-6">
                    <span class="cv-filterlabel">"Tags"</span>
                    <TagFilterMenu
                        tags=tags
                        on_change=Callback::new(move |t| set_ftags.set(t))
                    />
                </div>
            </div>

            <div class="cv-fs-sm cv-fg-faint cv-mb-10">
                {move || format!("{} decisions", count())}
            </div>
            <div class="cv-log">{rows}</div>
        </div>
    }
}
