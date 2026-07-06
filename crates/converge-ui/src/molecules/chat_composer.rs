use crate::atoms::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// The Expert-chat input box with a send button. Holds the draft internally and
/// fires `on_send` with the typed text — on the send button or on Enter (Shift+
/// Enter inserts a newline) — then clears the field. Empty/whitespace is ignored.
#[component]
pub fn ChatComposer(
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] on_send: Option<Callback<String>>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());
    // `Callback` is `Copy`, so the same `fire` can drive both the button and Enter.
    let fire = Callback::new(move |_: ()| {
        let t = text.get_untracked();
        if t.trim().is_empty() {
            return;
        }
        if let Some(cb) = on_send {
            cb.run(t);
        }
        set_text.set(String::new());
    });
    let on_key = move |ev: KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            fire.run(());
        }
    };
    view! {
        <div class="cv-composer">
            <input
                class="cv-composer__input"
                placeholder=placeholder
                prop:value=move || text.get()
                on:input=move |ev| set_text.set(event_target_value(&ev))
                on:keydown=on_key
            />
            <div class="cv-composer__send" on:click=move |_| fire.run(())>
                {Glyph::Send.glyph()}
            </div>
        </div>
    }
}
