use crate::atoms::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// A faint "＋ …" affordance row — e.g. "New project" under the sidebar's
/// project list. Brightens on hover; the whole row is the click target.
#[component]
pub fn AddRow(
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
        <div class="cv-addrow" role="button" tabindex="0" on:click=click on:keydown=keydown>
            <span class="cv-addrow__plus">{Glyph::Plus.glyph()}</span>
            <span>{label}</span>
        </div>
    }
}
