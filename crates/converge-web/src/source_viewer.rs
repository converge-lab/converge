//! The SourceViewer — the transcript a decision was captured from, with the
//! extracted passage highlighted. "Nothing is taken on faith."

use crate::data;
use crate::route::Route;
use converge_ui::atoms::Glyph;
use converge_ui::domain::SourceView;
use converge_ui::molecules::ConversationLine;
use leptos::prelude::*;

#[component]
pub fn SourceViewer(go: Callback<Route>, id: String, idx: usize) -> impl IntoView {
    let back_id = id.clone();
    let back = move |_| go.run(Route::Decision(back_id.clone()));
    let Some(d) = data::by_id(&id) else {
        return view! { <div class="cv-back" on:click=back>"← Back to decision"</div> }.into_any();
    };
    // An out-of-range index is a bad/stale deep link: show the back link rather
    // than substitute a different source and falsely claim it "Verified".
    let Some(src) = d.sources.get(idx) else {
        return view! { <div class="cv-back" on:click=back>"← Back to decision"</div> }.into_any();
    };
    let SourceView {
        kind,
        label,
        when,
        lines,
    } = data::to_source_view(src);
    let label_foot = label.clone();
    view! {
        <div class="cv-page">
            <div class="cv-back" on:click=back>"← Back to decision"</div>

            <div class="cv-row cv-gap-9 cv-mb-6">
                <span class="cv-fg-primary cv-fs-xl">{Glyph::Verified.glyph()}</span>
                <h1 class="cv-heading cv-fs-3xl">
                    "This decision was captured from here"
                </h1>
            </div>
            <p class="cv-fs-lg cv-fg-muted cv-mb-22 cv-lh-normal">
                "The highlighted passage is the exact source the decision was derived from. Read it in context — nothing is taken on faith."
            </p>

            <div class="cv-srchdr">
                <div class="cv-srchdr__tile">{kind.icon().glyph()}</div>
                <div class="cv-grow">
                    <div class="cv-fs-lg cv-fw-medium">
                        {kind.label()}" · "
                        <span class="cv-mono cv-fg-secondary">{label}</span>
                    </div>
                    <div class="cv-fs-xs cv-fg-faint">{when}</div>
                </div>
                <span class="cv-mono cv-fs-xs cv-fg-faint">"read-only"</span>
            </div>

            <div class="cv-conv">
                {lines.into_iter().map(|l| view! { <ConversationLine line=l /> }).collect_view()}
            </div>

            <div class="cv-verified">
                <span class="cv-verified__check">{Glyph::Verified.glyph()}</span>
                <div class="cv-fs-md cv-fg-primary">
                    "Verified — the decision matches the highlighted passage in "
                    <span class="cv-mono">{label_foot}</span> "."
                </div>
            </div>
        </div>
    }
    .into_any()
}
