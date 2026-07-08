//! The MCP connect command with a one-tap Copy button — shown on the
//! onboarding screen and the dashboard's empty-feed hint.

use leptos::prelude::*;

/// A mono command line plus a ghost "Copy" button that flips to "Copied ✓"
/// for 1.5s. Owns its own `copied` state; writes to the clipboard on the
/// browser, a no-op on native (compile-only) targets.
#[component]
pub fn CommandSnippet(#[prop(into)] command: String) -> impl IntoView {
    let (copied, set_copied) = signal(false);
    let to_copy = command.clone();
    let on_copy = move |_| {
        copy_to_clipboard(&to_copy);
        set_copied.set(true);
        reset_copied(set_copied);
    };
    view! {
        <div class="cv-onboard__snippet">
            <span class="cv-onboard__cmd">{command}</span>
            <button type="button" class="cv-btn cv-btn--ghost cv-btn--neutral" on:click=on_copy>
                <span>{move || if copied.get() { "Copied ✓" } else { "Copy" }}</span>
            </button>
        </div>
    }
}

#[cfg(target_arch = "wasm32")]
fn copy_to_clipboard(text: &str) {
    // Fire-and-forget: the async write resolves off-thread; the label flip is
    // optimistic. `navigator.clipboard` is present in every browser we target.
    let _ = window().navigator().clipboard().write_text(text);
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_to_clipboard(_text: &str) {}

#[cfg(target_arch = "wasm32")]
fn reset_copied(set_copied: WriteSignal<bool>) {
    set_timeout(
        move || set_copied.set(false),
        std::time::Duration::from_millis(1500),
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn reset_copied(_set_copied: WriteSignal<bool>) {}

/// The MCP connect command, pointed at this deployment's own origin — in
/// production the server serves the app and `/mcp` same-origin, so the current
/// host is the right one.
pub fn mcp_command() -> String {
    format!(
        "claude mcp add --transport http converge https://{}/mcp",
        mcp_host()
    )
}

#[cfg(target_arch = "wasm32")]
fn mcp_host() -> String {
    window()
        .location()
        .host()
        .unwrap_or_else(|_| "converge.internal".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn mcp_host() -> String {
    "converge.internal".into()
}
