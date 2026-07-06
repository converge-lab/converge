use crate::atoms::{CountBadge, Glyph, Icon};
use crate::domain::Tone;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// A sidebar navigation row: icon + label + optional count, with an active
/// state. `accent` tints the icon and count (e.g. Signals = amber).
#[component]
pub fn NavItem(
    icon: Glyph,
    #[prop(into)] label: String,
    #[prop(optional)] count: Option<u32>,
    #[prop(optional)] active: bool,
    #[prop(optional)] accent: Tone,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let class = if active {
        "cv-nav cv-nav--active"
    } else {
        "cv-nav"
    };
    let click = move |_| {
        if let Some(cb) = on_click {
            cb.run(());
        }
    };
    // A `<div on:click>` is invisible to keyboard/screen-reader users; the
    // role+tabindex+keydown trio is the standard retrofit for a custom control
    // that can't become a real <button> without a style rewrite.
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
            <span class="cv-nav__icon">
                <Icon glyph=icon size=14 color=accent.color_var() />
            </span>
            <span class="cv-nav__label">{label}</span>
            {count.map(move |c| view! { <CountBadge count=c tone=accent /> })}
        </div>
    }
}
