use crate::domain::ConvLine;
use leptos::prelude::*;

/// One line of a source transcript: speaker + text. The extracted passage is
/// highlighted and tagged.
#[component]
pub fn ConversationLine(line: ConvLine) -> impl IntoView {
    let ConvLine {
        speaker,
        text,
        extracted,
    } = line;
    let class = if extracted {
        "cv-conv__line cv-conv__line--hl"
    } else {
        "cv-conv__line"
    };
    view! {
        <div class=class>
            <div class="cv-conv__speaker">{speaker}</div>
            <div class="cv-conv__text">
                {text}
                {extracted.then(|| view! { <span class="cv-conv__tag">"▸ extracted"</span> })}
            </div>
        </div>
    }
}
