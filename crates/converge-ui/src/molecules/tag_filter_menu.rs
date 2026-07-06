use crate::atoms::Glyph;
use leptos::prelude::*;

/// A multi-select tag filter: a button that opens a checkbox menu. Holds the
/// selection internally and reports it through `on_change`.
#[component]
pub fn TagFilterMenu(
    tags: Vec<String>,
    /// Fired with the full selected-tag set whenever it changes.
    #[prop(optional, into)]
    on_change: Option<Callback<Vec<String>>>,
) -> impl IntoView {
    let (open, set_open) = signal(false);
    let (selected, set_selected) = signal::<Vec<String>>(Vec::new());
    let menu_tags = tags.clone();
    view! {
        <div class="cv-tagmenu">
            <div class="cv-tagmenu__btn" on:click=move |_| set_open.update(|o| *o = !*o)>
                <span>
                    {move || {
                        let n = selected.get().len();
                        if n == 0 { "All tags".to_string() } else { format!("{n} tags") }
                    }}
                </span>
                <span class="cv-tagmenu__caret">{Glyph::CaretDown.glyph()}</span>
            </div>
            {move || {
                let tags = menu_tags.clone();
                open.get()
                    .then(move || {
                        view! {
                            <div class="cv-tagmenu__menu">
                                {tags
                                    .into_iter()
                                    .map(|t| {
                                        let t_click = t.clone();
                                        let t_check = t.clone();
                                        view! {
                                            <div
                                                class="cv-tagmenu__item"
                                                on:click=move |_| {
                                                    let tag = t_click.clone();
                                                    set_selected
                                                        .update(|s| {
                                                            if let Some(i) = s.iter().position(|x| x == &tag) {
                                                                s.remove(i);
                                                            } else {
                                                                s.push(tag);
                                                            }
                                                        });
                                                    if let Some(cb) = on_change {
                                                        cb.run(selected.get_untracked());
                                                    }
                                                }
                                            >
                                                <span class="cv-tagmenu__check">
                                                    {move || {
                                                        if selected.get().contains(&t_check) {
                                                            Glyph::Verified.glyph()
                                                        } else {
                                                            ""
                                                        }
                                                    }}
                                                </span>
                                                <span class="cv-mono cv-fs-sm">{t}</span>
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        </div>
    }
}
