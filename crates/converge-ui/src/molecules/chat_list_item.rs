use crate::atoms::Glyph;
use leptos::ev::MouseEvent;
use leptos::prelude::*;

/// A chat row in the Expert panel: title (selects the chat) + a delete ✕. The
/// ✕ fires `on_delete` and stops propagation so it doesn't also select the row.
#[component]
pub fn ChatListItem(
    #[prop(into)] title: String,
    #[prop(optional)] active: bool,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
    #[prop(optional, into)] on_delete: Option<Callback<()>>,
) -> impl IntoView {
    let class = if active {
        "cv-chatitem cv-chatitem--active"
    } else {
        "cv-chatitem"
    };
    let click = move |_| {
        if let Some(cb) = on_click {
            cb.run(());
        }
    };
    let delete = move |ev: MouseEvent| {
        ev.stop_propagation();
        if let Some(cb) = on_delete {
            cb.run(());
        }
    };
    view! {
        <div class=class on:click=click>
            <span class="cv-chatitem__title">{title}</span>
            <span class="cv-chatitem__x" on:click=delete>{Glyph::Close.glyph()}</span>
        </div>
    }
}
