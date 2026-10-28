use crate::atoms::Badge;
use crate::domain::{Decision, Status};
use crate::molecules::AvatarStack;
use leptos::prelude::*;

/// The "recently captured" card: author stack + project + status + date,
/// then title and provenance. The most reused decision surface.
#[component]
pub fn DecisionCard(
    decision: Decision,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let Decision {
        authors,
        project,
        status,
        date,
        title,
        provenance,
        authors_label,
    } = decision;
    let show_status = status != Status::Accepted;
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    view! {
        <div class="cv-card" on:click=click>
            <div class="cv-card__meta">
                <AvatarStack authors=authors size=16 max=3 />
                <span class="cv-card__proj">{project}</span>
                {show_status.then(move || view! { <Badge label=status.label() tone=status.tone() /> })}
                <span class="cv-spacer"></span>
                <span class="cv-card__date">{date}</span>
            </div>
            <div class="cv-card__title">{title}</div>
            <div class="cv-card__prov">
                {authors_label}" · "<span class="cv-mono">{provenance}</span>
            </div>
        </div>
    }
}
