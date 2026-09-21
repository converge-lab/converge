use crate::atoms::Glyph;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// The Expert-chat input box with a send button. Holds the draft internally and
/// fires `on_send` on the send button or Enter, then clears the field. While a
/// reply is pending, the user can draft the next message without sending it.
#[component]
pub fn ChatComposer(
    #[prop(optional, into)] placeholder: Signal<String>,
    #[prop(optional, into)] pending: Signal<bool>,
    #[prop(optional, into)] on_send: Option<Callback<String>>,
) -> impl IntoView {
    let (text, set_text) = signal(String::new());
    // `Callback` is `Copy`, so the same `fire` can drive both the button and Enter.
    let fire = Callback::new(move |_: ()| {
        let t = text.get_untracked();
        if pending.get_untracked() || t.trim().is_empty() {
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
                aria-label="Message to expert"
                placeholder=placeholder
                prop:value=move || text.get()
                on:input=move |ev| set_text.set(event_target_value(&ev))
                on:keydown=on_key
            />
            <button
                type="button"
                class="cv-composer__send"
                aria-label="Send message"
                disabled=pending
                on:click=move |_| fire.run(())
            >
                {Glyph::Send.glyph()}
            </button>
        </div>
    }
}
