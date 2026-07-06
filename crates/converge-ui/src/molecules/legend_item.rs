use crate::domain::Tone;
use leptos::prelude::*;

/// A legend row: a tone dot + bold term + description.
#[component]
pub fn LegendItem(
    tone: Tone,
    #[prop(into)] term: String,
    #[prop(into)] desc: String,
) -> impl IntoView {
    view! {
        <div class="cv-legend">
            <span class="cv-legend__dot" style=format!("background:{}", tone.color_var())></span>
            <span><span class="cv-legend__term">{term}</span>" — "{desc}</span>
        </div>
    }
}
