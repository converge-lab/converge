use crate::domain::Source;
use leptos::prelude::*;

/// An anchored-evidence row: kind tile + "kind · label" + when + View source.
#[component]
pub fn SourceRow(
    source: Source,
    #[prop(optional, into)] on_view: Option<Callback<()>>,
) -> impl IntoView {
    let Source { kind, label, when } = source;
    let click = move |_| {
        if let Some(cb) = on_view {
            cb.run(());
        }
    };
    view! {
        <div class="cv-source" on:click=click>
            <div class="cv-source__tile">{kind.icon().glyph()}</div>
            <div class="cv-source__meta">
                <div class="cv-source__title">
                    {kind.label()}" · "<span class="cv-mono">{label}</span>
                </div>
                <div class="cv-source__when">{when}</div>
            </div>
            <div class="cv-source__btn">"View source"</div>
        </div>
    }
}
