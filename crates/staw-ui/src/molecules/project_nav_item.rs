use crate::atoms::CountBadge;
use crate::domain::Tone;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// A project row in the sidebar: mono name + unread count. Unread reads brighter.
#[component]
pub fn ProjectNavItem(
    #[prop(into)] name: String,
    #[prop(optional)] unread: u32,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let class = if unread > 0 {
        "cv-proj cv-proj--unread"
    } else {
        "cv-proj"
    };
    let click = move |_| {
        if let Some(cb) = on_click {
            cb.run(());
        }
    };
    let keydown = move |ev: KeyboardEvent| {
        if ev.key() == "Enter" || ev.key() == " " {
            ev.prevent_default();
            if let Some(cb) = on_click {
                cb.run(());
            }
        }
    };
    view! {
        <div class=class role="button" tabindex="0" on:click=click on:keydown=keydown>
            <span class="cv-truncate">{name}</span>
            {(unread > 0).then(move || view! { <CountBadge count=unread tone=Tone::Primary /> })}
        </div>
    }
}
