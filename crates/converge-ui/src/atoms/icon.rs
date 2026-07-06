use leptos::prelude::*;

/// The named glyph vocabulary. One place to map meaning → character today, and
/// the single swap point for an SVG icon set later. No raw glyphs at call sites.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Dashboard,
    Search,
    Signal,
    Expert,
    Verified,
    Close,
    Settings,
    Power,
    CaretDown,
    ChevronRight,
    ArrowRight,
    Send,
    Consequence,
    Supersede,
    Alternative,
    Warning,
    Plus,
    More,
    Personal,
    Shared,
    Brand,
    Transcript,
    Slack,
    Pr,
}

impl Glyph {
    pub fn glyph(self) -> &'static str {
        match self {
            Glyph::Dashboard => "▤",
            Glyph::Search => "⌕",
            Glyph::Signal => "⇄",
            Glyph::Expert => "✦",
            Glyph::Verified => "✓",
            Glyph::Close => "✕",
            Glyph::Settings => "⚙",
            Glyph::Power => "⏻",
            Glyph::CaretDown => "▾",
            Glyph::ChevronRight => "›",
            Glyph::ArrowRight => "→",
            Glyph::Send => "↑",
            Glyph::Consequence => "↗",
            Glyph::Supersede => "↻",
            Glyph::Alternative => "◑",
            Glyph::Warning => "!",
            Glyph::Plus => "＋",
            Glyph::More => "⋯",
            Glyph::Personal => "◐",
            Glyph::Shared => "⧉",
            Glyph::Brand => "◆",
            Glyph::Transcript => "“”",
            Glyph::Slack => "#",
            Glyph::Pr => "⎇",
        }
    }
}

/// A single glyph from the typed vocabulary.
#[component]
pub fn Icon(
    glyph: Glyph,
    #[prop(default = 14)] size: u32,
    /// Optional CSS colour; omit to inherit.
    #[prop(optional, into)]
    color: String,
) -> impl IntoView {
    let mut style = format!("font-size:{}rem;", size as f64 / 16.0);
    if !color.is_empty() {
        style.push_str(&format!("color:{color};"));
    }
    view! { <span class="cv-icon" style=style>{glyph.glyph()}</span> }
}
