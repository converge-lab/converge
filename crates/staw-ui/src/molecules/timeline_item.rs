use crate::atoms::Badge;
use crate::domain::ChainNode;
use leptos::prelude::*;

/// One node of a supersession timeline: dot + connector + card. `current`
/// highlights the live decision; `last` drops the trailing connector line.
#[component]
pub fn TimelineItem(
    node: ChainNode,
    #[prop(optional)] last: bool,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let ChainNode {
        title,
        project,
        date,
        status,
        current,
    } = node;
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    let dot_class = if current {
        "cv-tl__dot cv-tl__dot--current"
    } else {
        "cv-tl__dot"
    };
    let card_class = if current {
        "cv-tl__card cv-tl__card--current"
    } else {
        "cv-tl__card"
    };
    view! {
        <div class="cv-tl">
            <div class="cv-tl__rail">
                <div class=dot_class></div>
                {(!last).then(|| view! { <div class="cv-tl__line"></div> })}
            </div>
            <div class=card_class on:click=click>
                <div class="cv-tl__head">
                    <span class="cv-tl__title">{title}</span>
                    <span class="cv-spacer"></span>
                    <Badge label=status.label() tone=status.tone() />
                </div>
                <div class="cv-tl__meta">{project}" · "{date}</div>
            </div>
        </div>
    }
}
