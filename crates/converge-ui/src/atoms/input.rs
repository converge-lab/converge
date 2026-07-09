use super::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// Text input with an optional leading glyph, an optional trailing keyboard
/// hint, and a focus ring. Emits the current value through `on_input`;
/// `value` is reactive (pass a signal to keep the field in sync, e.g. to
/// clear it after a submit), and `on_keydown` lets callers wire Enter-to-
/// submit without re-implementing the input chrome.
#[component]
pub fn Input(
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] lead: Option<Glyph>,
    #[prop(optional, into)] value: Option<Signal<String>>,
    #[prop(optional, into)] trail: String,
    #[prop(optional, into)] on_input: Option<Callback<String>>,
    #[prop(optional, into)] on_keydown: Option<Callback<KeyboardEvent>>,
) -> impl IntoView {
    let has_trail = !trail.is_empty();
    let oninput = move |ev| {
        if let Some(cb) = on_input {
            cb.run(event_target_value(&ev));
        }
    };
    let onkeydown = move |ev: KeyboardEvent| {
        if let Some(cb) = on_keydown {
            cb.run(ev);
        }
    };
    view! {
        <div class="cv-input">
            {lead.map(|g| view! { <span class="cv-input__lead">{g.glyph()}</span> })}
            <input
                class="cv-input__field"
                prop:value=move || value.map(|v| v.get()).unwrap_or_default()
                placeholder=placeholder
                on:input=oninput
                on:keydown=onkeydown
            />
            {has_trail.then(move || view! { <span class="cv-input__kbd">{trail}</span> })}
        </div>
    }
}
