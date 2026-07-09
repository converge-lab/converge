use crate::atoms::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// A choice card on the onboarding screen: a glyph tile above a title and a
/// one-line description. The whole card is the click target.
#[component]
pub fn OnboardCard(
    glyph: Glyph,
    #[prop(into)] title: String,
    #[prop(into)] desc: String,
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
        <div class="cv-onboard__card" role="button" tabindex="0" on:click=click on:keydown=keydown>
            <div class="cv-onboard__glyph">{glyph.glyph()}</div>
            <div class="cv-onboard__title">{title}</div>
            <div class="cv-onboard__desc">{desc}</div>
        </div>
    }
}
