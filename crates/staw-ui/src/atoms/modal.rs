use leptos::ev::MouseEvent;
use leptos::prelude::*;

/// A centred modal over a dimming scrim. Clicking the scrim closes it (through
/// `on_close`); clicks inside the panel don't. Renders the title/subtitle
/// header; the body — fields, chips, footer — is the component's children.
/// Escape-to-close and Enter-to-submit live on the caller's inputs (matching
/// the prototype), so an autofocused field drives both.
#[component]
pub fn Modal(
    #[prop(into)] title: String,
    #[prop(into)] subtitle: String,
    on_close: Callback<()>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="cv-modal-scrim" on:click=move |_| on_close.run(())>
            <div class="cv-modal" on:click=|ev: MouseEvent| ev.stop_propagation()>
                <div class="cv-col cv-gap-6">
                    <div class="cv-fs-xl cv-fw-semibold">{title}</div>
                    <div class="cv-fs-md cv-fg-muted">{subtitle}</div>
                </div>
                {children()}
            </div>
        </div>
    }
}
