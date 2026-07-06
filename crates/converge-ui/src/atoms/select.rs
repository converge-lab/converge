use leptos::prelude::*;

/// Native select styled to the system. Options are `(value, label)` pairs.
#[component]
pub fn Select(
    options: Vec<(String, String)>,
    #[prop(optional, into)] value: String,
    #[prop(optional, into)] on_change: Option<Callback<String>>,
) -> impl IntoView {
    let onchange = move |ev| {
        if let Some(cb) = on_change {
            cb.run(event_target_value(&ev));
        }
    };
    // Bind `value` only when one is given; otherwise let the browser default to
    // the first <option> instead of showing a blank box.
    let bound = (!value.is_empty()).then_some(value);
    view! {
        <select class="cv-select" prop:value=bound on:change=onchange>
            {options
                .into_iter()
                .map(|(v, l)| view! { <option value=v>{l}</option> })
                .collect_view()}
        </select>
    }
}
