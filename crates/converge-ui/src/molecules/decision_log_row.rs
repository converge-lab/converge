use crate::atoms::Badge;
use crate::domain::DecisionRef;
use crate::molecules::AvatarStack;
use leptos::prelude::*;

/// A row in a project's decision log table: unread dot + avatars + title/summary
/// + status + date. Denser than `DecisionMiniRow`; lives inside a `.cv-log`.
#[component]
pub fn DecisionLogRow(
    decision: DecisionRef,
    #[prop(into)] date: String,
    #[prop(optional)] unread: bool,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let DecisionRef {
        authors,
        status,
        title,
        summary,
        ..
    } = decision;
    let dot_class = if unread {
        "cv-logrow__dot cv-logrow__dot--on"
    } else {
        "cv-logrow__dot"
    };
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    view! {
        <div class="cv-logrow" on:click=click>
            <span class=dot_class></span>
            <AvatarStack authors=authors size=23 max=2 />
            <div class="cv-logrow__body">
                <div class="cv-logrow__title">{title}</div>
                <div class="cv-logrow__summary">{summary}</div>
            </div>
            <span class="cv-logrow__status">
                <Badge label=status.label() tone=status.tone() />
            </span>
            <span class="cv-logrow__date">{date}</span>
        </div>
    }
}
