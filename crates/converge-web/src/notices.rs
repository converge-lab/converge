//! A compact view of retained outcomes. Folding never discards a failure.

use converge_ui::atoms::Glyph;
use leptos::prelude::*;

use crate::store::{AppStateStoreFields, AppStore, dismiss_notice};

const VISIBLE_NOTICES: usize = 5;

#[component]
pub fn Notices(store: AppStore) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let count = move || store.notices().with(Vec::len);
    view! {
        <div
            class="cv-toasts"
            class:cv-toasts--expanded={move || expanded.get() && count() > VISIBLE_NOTICES}
            role="region"
            aria-label="Notifications"
        >
            <For
                each=move || store.notices().with(|notices| {
                    let skip = if expanded.get() { 0 } else { notices.len().saturating_sub(VISIBLE_NOTICES) };
                    notices.iter().skip(skip).cloned().collect::<Vec<_>>()
                })
                key=|entry| entry.id
                children=move |entry| {
                    let id = entry.id;
                    let ok = entry.notice.is_ok();
                    let occurrences = move || store.notices().with(|notices| {
                        notices.iter().find(|n| n.id == id).map_or(1, |n| n.occurrences)
                    });
                    view! {
                        <div class=if ok { "cv-toast cv-toast--ok" } else { "cv-toast" }
                            role=if ok { "status" } else { "alert" }>
                            <span class="cv-grow">{entry.notice.text().to_string()}</span>
                            <span class="cv-toast__count" hidden=move || occurrences() == 1
                                aria-label=move || format!("Repeated {} times", occurrences())>
                                {move || format!("×{}", occurrences())}
                            </span>
                            <button type="button" class="cv-toast__close" aria-label="Dismiss"
                                on:click=move |_| dismiss_notice(store, id)>
                                {Glyph::Close.glyph()}
                            </button>
                        </div>
                    }
                }
            />
            {move || (count() > VISIBLE_NOTICES).then(|| view! {
                <button type="button" class="cv-toasts__toggle"
                    aria-expanded=move || expanded.get().to_string()
                    on:click=move |_| expanded.update(|value| *value = !*value)>
                    {move || if expanded.get() {
                        "Show fewer notifications".to_string()
                    } else {
                        format!("Show all {} notifications", count())
                    }}
                </button>
            })}
        </div>
    }
}
