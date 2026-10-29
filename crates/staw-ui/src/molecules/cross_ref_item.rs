use crate::domain::CrossRef;
use leptos::prelude::*;

/// A reference to this decision from another project: project tag + title + why.
#[component]
pub fn CrossRefItem(
    cross_ref: CrossRef,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let CrossRef {
        project,
        title,
        why,
    } = cross_ref;
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    view! {
        <div class="cv-xref" on:click=click>
            <div class="cv-xref__head">
                <span class="cv-xref__proj">{project}</span>
                <span class="cv-xref__title">{title}</span>
            </div>
            {why.map(|w| view! { <div class="cv-xref__why">{w}</div> })}
        </div>
    }
}
