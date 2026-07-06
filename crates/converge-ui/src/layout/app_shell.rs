use leptos::prelude::*;

/// The app frame: a fixed sidebar plus a main column (top bar + scroll area).
/// `sidebar` and `topbar` are passed as already-rendered views; the page body
/// is the component's children. The theme is inherited from the document root
/// (`<html data-theme=…>`), so the account-menu toggle themes the whole app.
#[component]
pub fn AppShell(sidebar: AnyView, topbar: AnyView, children: Children) -> impl IntoView {
    view! {
        <div class="cv-shell">
            {sidebar}
            <div class="cv-shell__main">
                <header class="cv-shell__topbar">{topbar}</header>
                <main class="cv-shell__scroll">{children()}</main>
            </div>
        </div>
    }
}
