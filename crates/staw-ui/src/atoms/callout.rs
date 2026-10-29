use super::Glyph;
use crate::domain::Tone;
use leptos::prelude::*;

/// Toned, optionally-titled box. Absorbs the prototype's five colored boxes:
/// "what happens", "recommended", "verified", "known to the expert", source-ok.
#[component]
pub fn Callout(
    #[prop(optional)] tone: Tone,
    #[prop(optional, into)] icon: Option<Glyph>,
    #[prop(optional, into)] title: String,
    children: Children,
) -> impl IntoView {
    let has_title = !title.is_empty();
    let has_head = icon.is_some() || has_title;
    view! {
        <div class=format!("cv-callout cv-callout--{}", tone.slug())>
            {has_head
                .then(move || {
                    view! {
                        <div class="cv-callout__head">
                            {icon.map(|g| view! { <span class="cv-callout__icon">{g.glyph()}</span> })}
                            {has_title.then(move || view! { <span class="cv-callout__title">{title}</span> })}
                        </div>
                    }
                })}
            <div class="cv-callout__body">{children()}</div>
        </div>
    }
}
