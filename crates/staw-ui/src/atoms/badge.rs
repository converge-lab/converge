use crate::domain::Tone;
use leptos::prelude::*;

/// Small label for status, risk, or accent. Absorbs the prototype's repeated
/// `statusColor` / `statusBg` / `riskBg` pills (~9 call-sites) via `tone`.
#[component]
pub fn Badge(
    #[prop(into)] label: String,
    #[prop(optional)] tone: Tone,
    /// Drop the filled background (used inline in dense rows).
    #[prop(optional)]
    subtle: bool,
) -> impl IntoView {
    let class = format!(
        "cv-badge cv-badge--{}{}",
        tone.slug(),
        if subtle { " cv-badge--subtle" } else { "" }
    );
    view! { <span class=class>{label}</span> }
}
