use super::Glyph;
use crate::domain::Tone;
use leptos::prelude::*;

/// The uppercase "eyebrow" section header — the single most repeated text
/// style in the prototype (~12 call-sites). `bar` adds a tone-coloured tick.
#[component]
pub fn SectionLabel(
    #[prop(into)] text: String,
    #[prop(optional, into)] icon: Option<Glyph>,
    #[prop(optional)] tone: Tone,
    #[prop(optional)] bar: bool,
) -> impl IntoView {
    // With a bar the whole label takes the tone colour (the prototype's
    // "Decision"); with an icon only the icon is toned and the text stays muted.
    let class = if bar {
        "cv-eyebrow cv-eyebrow--bar"
    } else {
        "cv-eyebrow"
    };
    view! {
        <div class=class data-tone=tone.slug()>
            {bar.then(|| view! { <span class="cv-eyebrow__bar"></span> })}
            {icon.map(|g| view! { <span class="cv-eyebrow__icon">{g.glyph()}</span> })}
            <span>{text}</span>
        </div>
    }
}
