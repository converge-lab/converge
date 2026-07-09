use crate::atoms::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// The floating panel of an overflow ("⋯") menu — it holds [`MenuItem`]s. The
/// trigger button, open state, and dismissal (outside-click / Escape) are the
/// caller's, since they depend on the anchor; this is just the panel.
#[component]
pub fn OverflowMenu(children: Children) -> impl IntoView {
    view! { <div class="cv-menu">{children()}</div> }
}

/// One actionable row in an [`OverflowMenu`]: a leading glyph and a label.
#[component]
pub fn MenuItem(
    icon: Glyph,
    #[prop(into)] label: String,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let click = move |_| {
        if let Some(cb) = on_click {
            cb.run(());
        }
    };
    let keydown = move |ev: KeyboardEvent| {
        if ev.key() == "Enter" || ev.key() == " " {
            ev.prevent_default();
            if let Some(cb) = on_click {
                cb.run(());
            }
        }
    };
    view! {
        <div class="cv-menu__item" role="button" tabindex="0" on:click=click on:keydown=keydown>
            <span class="cv-menu__icon">{icon.glyph()}</span>
            {label}
        </div>
    }
}
