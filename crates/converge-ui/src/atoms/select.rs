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
    // Selection is expressed on the matching <option> rather than as a
    // `prop:value` on the <select>: the value property applies before the
    // options exist (and even `None` writes it), leaving selectedIndex -1 —
    // a blank box. With no `value` the browser defaults to the first option.
    let bound = (!value.is_empty()).then_some(value);
    view! {
        <select class="cv-select" on:change=onchange>
            {options
                .into_iter()
                .map(|(v, l)| {
                    let selected = bound.as_deref() == Some(v.as_str());
                    view! { <option value=v selected=selected>{l}</option> }
                })
                .collect_view()}
        </select>
    }
}
