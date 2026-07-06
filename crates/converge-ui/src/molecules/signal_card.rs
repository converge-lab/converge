use crate::atoms::{Badge, Glyph};
use crate::domain::Signal;
use leptos::prelude::*;

/// How a `SignalCard` is presented. The prototype shows the same signal three
/// ways: a compact dashboard tile, the full Signals-page row, and a brief
/// "related" card on a decision's detail.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalView {
    /// Dashboard side-panel: route + lowercase badge + text, no bar/title/footer.
    #[default]
    Compact,
    /// Signals page: risk bar + title + text + "N source decisions" footer.
    Full,
    /// Decision detail "related signals": risk bar + title only (no text/footer).
    Related,
}

/// Cross-project signal: `from → to` + risk badge. The three `SignalView`s mirror
/// the prototype's dashboard / signals-page / decision-detail presentations.
#[component]
pub fn SignalCard(
    signal: Signal,
    #[prop(optional)] view: SignalView,
    #[prop(optional)] source_count: Option<u32>,
    #[prop(optional, into)] on_open: Option<Callback<()>>,
) -> impl IntoView {
    let Signal {
        from,
        to,
        risk,
        title,
        text,
    } = signal;
    let click = move |_| {
        if let Some(cb) = on_open {
            cb.run(());
        }
    };
    let bar = view != SignalView::Compact;
    let show_title = view != SignalView::Compact;
    let show_text = view != SignalView::Related;
    let show_footer = view == SignalView::Full;
    let style = if bar {
        format!("border-left:3px solid {}", risk.tone().color_var())
    } else {
        String::new()
    };
    // The dashboard tile uses a lowercase label; the bar variants capitalise it.
    let label = if view == SignalView::Compact {
        risk.label().to_lowercase()
    } else {
        risk.label().to_string()
    };
    view! {
        <div class="cv-signal" style=style on:click=click>
            <div class="cv-signal__route">
                <span>{from}</span>
                <span class="cv-signal__arrow">{Glyph::ArrowRight.glyph()}</span>
                <span>{to}</span>
                <span class="cv-spacer"></span>
                <Badge label=label tone=risk.tone() />
            </div>
            {show_title.then(move || view! { <div class="cv-signal__title">{title}</div> })}
            {show_text.then(move || view! { <div class="cv-signal__text">{text}</div> })}
            {show_footer
                .then(move || {
                    view! {
                        <div class="cv-signal__foot">
                            <span class="cv-mono">
                                {source_count
                                    .map(|n| match n {
                                        1 => "1 source decision".to_string(),
                                        _ => format!("{n} source decisions"),
                                    })
                                    .unwrap_or_default()}
                            </span>
                            <span class="cv-spacer"></span>
                            <span class="cv-signal__view">"View signal " {Glyph::ArrowRight.glyph()}</span>
                        </div>
                    }
                })}
        </div>
    }
}
