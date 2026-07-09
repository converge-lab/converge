//! Copy-to-clipboard pieces: the shared [`CopyButton`] and the MCP connect
//! [`CommandSnippet`] built on it (the onboarding screen, the dashboard's
//! empty-feed hint, and the token reveal on Settings all copy something).

use leptos::prelude::*;

/// A ghost "Copy" button that writes `text` to the clipboard and flips its
/// label to "Copied ✓" for 1.5s. Uses `navigator.clipboard` where available
/// and falls back to a hidden textarea + `execCommand("copy")` on insecure
/// origins (plain http beyond localhost), where the clipboard API is absent.
#[component]
pub fn CopyButton(#[prop(into)] text: String) -> impl IntoView {
    let (copied, set_copied) = signal(false);
    let on_copy = move |_| {
        copy_to_clipboard(&text);
        set_copied.set(true);
        reset_copied(set_copied);
    };
    view! {
        <button type="button" class="cv-btn cv-btn--ghost cv-btn--neutral" on:click=on_copy>
            <span>{move || if copied.get() { "Copied ✓" } else { "Copy" }}</span>
        </button>
    }
}

/// A mono command line plus a [`CopyButton`].
#[component]
pub fn CommandSnippet(#[prop(into)] command: String) -> impl IntoView {
    view! {
        <div class="cv-onboard__snippet">
            <span class="cv-onboard__cmd">{command.clone()}</span>
            <CopyButton text=command />
        </div>
    }
}

#[cfg(target_arch = "wasm32")]
fn copy_to_clipboard(text: &str) {
    use wasm_bindgen::JsCast;
    // Fire-and-forget: the async write resolves off-thread; the label flip is
    // optimistic.
    let navigator = window().navigator();
    let has_clipboard = js_sys::Reflect::get(navigator.as_ref(), &"clipboard".into())
        .map(|v| !v.is_undefined())
        .unwrap_or(false);
    if has_clipboard {
        let _ = navigator.clipboard().write_text(text);
        return;
    }
    // Insecure-origin fallback: `navigator.clipboard` only exists in secure
    // contexts, so select-and-copy through an off-screen textarea instead.
    let doc = document();
    let Ok(el) = doc.create_element("textarea") else {
        return;
    };
    let _ = el.set_attribute("style", "position:fixed;top:0;left:0;opacity:0");
    let Ok(ta) = el.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return;
    };
    ta.set_value(text);
    if let Some(body) = doc.body() {
        let _ = body.append_child(&ta);
        ta.select();
        // `execCommand` lives on HtmlDocument in the web-sys bindings.
        if let Some(html_doc) = doc.dyn_ref::<web_sys::HtmlDocument>() {
            let _ = html_doc.exec_command("copy");
        }
        let _ = body.remove_child(&ta);
    }
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
/// production the server serves the app and `/mcp` same-origin, so the page's
/// scheme *and* host are exactly the ones an agent must connect to (an http
/// deployment must not be told to speak TLS).
pub fn mcp_command() -> String {
    format!(
        "claude mcp add --transport http converge {}/mcp",
        mcp_origin()
    )
}

#[cfg(target_arch = "wasm32")]
fn mcp_origin() -> String {
    window()
        .location()
        .origin()
        .unwrap_or_else(|_| "https://converge.internal".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn mcp_origin() -> String {
    "https://converge.internal".into()
}
