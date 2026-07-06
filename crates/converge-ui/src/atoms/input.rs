use super::Glyph;
use leptos::prelude::*;

/// Text input with an optional leading glyph, an optional trailing keyboard
/// hint, and a focus ring. Emits the current value through `on_input`.
#[component]
pub fn Input(
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] lead: Option<Glyph>,
    #[prop(optional, into)] value: String,
    #[prop(optional, into)] trail: String,
    #[prop(optional, into)] on_input: Option<Callback<String>>,
) -> impl IntoView {
    let has_trail = !trail.is_empty();
    let oninput = move |ev| {
        if let Some(cb) = on_input {
            cb.run(event_target_value(&ev));
        }
    };
    view! {
        <div class="cv-input">
            {lead.map(|g| view! { <span class="cv-input__lead">{g.glyph()}</span> })}
            <input
                class="cv-input__field"
                prop:value=value
                placeholder=placeholder
                on:input=oninput
            />
            {has_trail.then(move || view! { <span class="cv-input__kbd">{trail}</span> })}
        </div>
    }
}
