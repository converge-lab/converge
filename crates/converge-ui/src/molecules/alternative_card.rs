use crate::domain::Alternative;
use leptos::prelude::*;

/// A rejected alternative: struck-through option + why it lost.
#[component]
pub fn AlternativeCard(alternative: Alternative) -> impl IntoView {
    let Alternative {
        option,
        why_rejected,
    } = alternative;
    view! {
        <div class="cv-alt">
            <div class="cv-alt__opt">{option}</div>
            <div class="cv-alt__why">{why_rejected}</div>
        </div>
    }
}
