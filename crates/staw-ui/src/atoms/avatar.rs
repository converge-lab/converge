use leptos::prelude::*;

/// Round author chip with initials. `ring` draws the surface-coloured halo
/// used when avatars overlap in a stack.
#[component]
pub fn Avatar(
    #[prop(into)] initial: String,
    /// Any CSS colour (token or literal), e.g. `"#3ecf8e"`.
    #[prop(into)]
    color: String,
    #[prop(default = 24)] size: u32,
    #[prop(optional)] ring: bool,
) -> impl IntoView {
    let style = format!(
        "--cv-avatar-size:{}rem;background:{color};",
        size as f64 / 16.0
    );
    let class = if ring {
        "cv-avatar cv-avatar--ring"
    } else {
        "cv-avatar"
    };
    view! { <span class=class style=style>{initial}</span> }
}
