//! Markdown rendering. Decision bodies are stored as markdown and the API
//! passes them through unchanged (good for the LLM reader too); this is where
//! the *human* render happens. We parse with the `markdown` crate (mdast) and
//! map the tree to Leptos views — no `inner_html`, so it's safe by construction
//! and themed with our tokens.

use leptos::prelude::*;
use markdown::mdast::Node;

/// Max nesting we'll recurse into. Bounds stack usage on adversarial input
/// (deeply nested lists/emphasis) so a hostile body can't overflow the WASM
/// stack; real decision bodies are nowhere near this.
const MAX_DEPTH: usize = 24;

/// Render a markdown string as themed, sanitised Leptos views.
#[component]
pub fn Markdown(#[prop(into)] source: String) -> impl IntoView {
    let inner = match markdown::to_mdast(&source, &markdown::ParseOptions::gfm()) {
        Ok(tree) => render(tree, 0),
        Err(_) => source.into_any(),
    };
    view! { <div class="cv-md">{inner}</div> }
}

fn render_children(children: Vec<Node>, depth: usize) -> AnyView {
    children
        .into_iter()
        .map(|c| render(c, depth))
        .collect_view()
        .into_any()
}

fn render(node: Node, depth: usize) -> AnyView {
    if depth > MAX_DEPTH {
        return ().into_any();
    }
    let d = depth + 1;
    match node {
        Node::Root(n) => render_children(n.children, d),
        Node::Paragraph(n) => {
            view! { <p class="cv-md-p">{render_children(n.children, d)}</p> }.into_any()
        }
        Node::Text(n) => n.value.into_any(),
        Node::Strong(n) => view! { <strong>{render_children(n.children, d)}</strong> }.into_any(),
        Node::Emphasis(n) => view! { <em>{render_children(n.children, d)}</em> }.into_any(),
        Node::InlineCode(n) => view! { <code class="cv-md-code">{n.value}</code> }.into_any(),
        Node::Link(n) => {
            let href = safe_href(n.url);
            view! {
                <a class="cv-md-link" href=href target="_blank" rel="noopener noreferrer">
                    {render_children(n.children, d)}
                </a>
            }
            .into_any()
        }
        Node::List(n) => {
            let ordered = n.ordered;
            let items = n
                .children
                .into_iter()
                .map(|c| view! { <li>{render(c, d)}</li> })
                .collect_view();
            if ordered {
                view! { <ol class="cv-md-ol">{items}</ol> }.into_any()
            } else {
                view! { <ul class="cv-md-ul">{items}</ul> }.into_any()
            }
        }
        Node::ListItem(n) => render_children(n.children, d),
        Node::Heading(n) => {
            let depth = (n.depth as usize).clamp(1, 6);
            // Keep the visual scale capped at 3 but emit the real heading tag so
            // the document outline / screen-reader navigation is preserved.
            let cls = format!("cv-md-h cv-md-h{}", depth.min(3));
            let inner = render_children(n.children, d);
            match depth {
                1 => view! { <h1 class=cls>{inner}</h1> }.into_any(),
                2 => view! { <h2 class=cls>{inner}</h2> }.into_any(),
                3 => view! { <h3 class=cls>{inner}</h3> }.into_any(),
                4 => view! { <h4 class=cls>{inner}</h4> }.into_any(),
                5 => view! { <h5 class=cls>{inner}</h5> }.into_any(),
                _ => view! { <h6 class=cls>{inner}</h6> }.into_any(),
            }
        }
        Node::Code(n) => view! { <pre class="cv-md-pre"><code>{n.value}</code></pre> }.into_any(),
        Node::Break(_) => view! { <br /> }.into_any(),
        // Unhandled nodes (images, tables, footnotes, …) render nothing for now.
        _ => ().into_any(),
    }
}

/// Allowlist link schemes. Browsers ignore ASCII whitespace and C0 control
/// characters when resolving a URL's scheme, so a denylist on the raw string is
/// bypassable (`java&Tab;script:` → `javascript:`). We therefore strip those
/// characters before reading the scheme and permit only known-safe schemes;
/// relative URLs (no scheme before the path) pass through unchanged.
fn safe_href(url: String) -> String {
    let probe: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    // The scheme is the text before the first ':' that comes before any path
    // delimiter; if a delimiter comes first (or there's no ':'), it's relative.
    let scheme = match probe.find([':', '/', '?', '#']) {
        Some(i) if probe.as_bytes()[i] == b':' => Some(&probe[..i]),
        _ => None,
    };
    match scheme {
        None => url,
        Some("http" | "https" | "mailto") => url,
        Some(_) => "#".into(),
    }
}
