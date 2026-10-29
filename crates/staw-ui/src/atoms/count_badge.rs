use crate::domain::Tone;
use leptos::prelude::*;

/// Small numeric pill — sidebar unread counts, signal counts.
#[component]
pub fn CountBadge(count: u32, #[prop(optional)] tone: Tone) -> impl IntoView {
    let class = format!("cv-count cv-count--{}", tone.slug());
    view! { <span class=class>{count}</span> }
}
