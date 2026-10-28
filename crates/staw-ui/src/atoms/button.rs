use super::Glyph;
use crate::domain::Tone;
use leptos::ev::MouseEvent;
use leptos::prelude::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Filled,
    Outline,
    Ghost,
}

impl ButtonVariant {
    fn slug(self) -> &'static str {
        match self {
            ButtonVariant::Filled => "filled",
            ButtonVariant::Outline => "outline",
            ButtonVariant::Ghost => "ghost",
        }
    }
}

/// Action button. `variant` × `tone` covers the prototype's filled send button,
/// the green "View source" outline, and ghost menu actions.
#[component]
pub fn Button(
    #[prop(into)] label: String,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] tone: Tone,
    #[prop(optional, into)] icon: Option<Glyph>,
    /// Reactive disabled state — dims the button and blocks clicks
    /// (`.cv-btn:disabled`). Used by modal submits gated on a valid field.
    #[prop(optional, into)]
    disabled: Signal<bool>,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let class = format!("cv-btn cv-btn--{} cv-btn--{}", variant.slug(), tone.slug());
    let click = move |_ev: MouseEvent| {
        if disabled.get() {
            return;
        }
        if let Some(cb) = on_click {
            cb.run(());
        }
    };
    view! {
        // Explicit `type="button"` so the component never accidentally submits a
        // surrounding form (the HTML default is `submit`).
        <button type="button" class=class disabled=disabled on:click=click>
            {icon.map(|g| view! { <span class="cv-btn__icon">{g.glyph()}</span> })}
            <span>{label}</span>
        </button>
    }
}
