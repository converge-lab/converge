use crate::atoms::Glyph;
use crate::domain::GroupKind;
use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

/// A group row in the sidebar's Groups list: kind glyph + name + how many
/// projects it holds. With no switcher above it, the selected row is both
/// "where you are" and "what else exists" — so the active state has to read as
/// selected, not merely hovered.
#[component]
pub fn GroupNavItem(
    #[prop(into)] name: String,
    kind: GroupKind,
    #[prop(optional)] projects: usize,
    #[prop(optional)] active: bool,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let class = if active {
        "cv-nav cv-nav--active"
    } else {
        "cv-nav"
    };
    let icon = match kind {
        GroupKind::Personal => Glyph::Personal,
        GroupKind::Shared => Glyph::Shared,
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
            <span class="cv-nav__icon">{icon.glyph()}</span>
            <span class="cv-nav__label cv-truncate">{name}</span>
            // A plain count, not a CountBadge: the badge means unread, and a
            // project tally is neither news nor an alert.
            <span class="cv-nav__count">{projects}</span>
        </div>
    }
}
