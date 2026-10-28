use crate::atoms::Badge;
use crate::domain::DecisionRef;
use crate::molecules::AvatarStack;
use leptos::prelude::*;

/// A compact decision row: avatars + title/summary + project + status. Reused
/// by signal sources and (later) search results.
#[component]
pub fn DecisionMiniRow(
    decision: DecisionRef,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let DecisionRef {
        authors,
        project,
        status,
        title,
        summary,
    } = decision;
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    view! {
        <div class="cv-mini" on:click=click>
            <AvatarStack authors=authors size=24 max=2 />
            <div class="cv-mini__body">
                <div class="cv-mini__title">{title}</div>
                <div class="cv-mini__summary">{summary}</div>
            </div>
            <div class="cv-mini__meta">
                <span class="cv-mini__proj cv-mono">{project}</span>
                <Badge label=status.label() tone=status.tone() />
            </div>
        </div>
    }
}
