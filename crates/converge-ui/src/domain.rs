//! Shared types + the semantic tones the atoms consume.
//!
//! `Status`/`Risk` own their (label, tone) mapping, which is what kills the
//! prototype's repeated inline `statusColor` / `statusBg` / `riskBg` strings.

/// The accent palette, by meaning rather than by colour.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    /// muted grey — default
    #[default]
    Neutral,
    /// green — verified / accepted / brand
    Primary,
    /// purple — expert model / agent
    Expert,
    /// amber — cross-project signal / draft
    Signal,
    /// red — will-break / destructive
    Danger,
}

impl Tone {
    pub fn slug(self) -> &'static str {
        match self {
            Tone::Neutral => "neutral",
            Tone::Primary => "primary",
            Tone::Expert => "expert",
            Tone::Signal => "signal",
            Tone::Danger => "danger",
        }
    }

    /// The CSS colour token for this tone — for inline `color:` on glyphs etc.
    pub fn color_var(self) -> &'static str {
        match self {
            Tone::Neutral => "var(--cv-text-muted)",
            Tone::Primary => "var(--cv-primary)",
            Tone::Expert => "var(--cv-expert)",
            Tone::Signal => "var(--cv-signal)",
            Tone::Danger => "var(--cv-danger)",
        }
    }
}

/// Lifecycle of a decision (ADR).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Accepted,
    Draft,
    Proposed,
    Superseded,
    Rejected,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Accepted => "accepted",
            Status::Draft => "draft",
            Status::Proposed => "proposed",
            Status::Superseded => "superseded",
            Status::Rejected => "rejected",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Status::Accepted => Tone::Primary,
            Status::Draft => Tone::Signal,
            Status::Proposed => Tone::Expert,
            Status::Superseded => Tone::Neutral,
            Status::Rejected => Tone::Danger,
        }
    }
}

/// Severity of a cross-project signal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    WillBreak,
    Coordinate,
    Watch,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::WillBreak => "Will break",
            Risk::Coordinate => "Coordinate",
            Risk::Watch => "Watch",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Risk::WillBreak => Tone::Danger,
            Risk::Coordinate => Tone::Signal,
            Risk::Watch => Tone::Expert,
        }
    }
}

/// Who (or what) authored a decision. Humans and agents render differently.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    Human,
    Agent,
}

#[derive(Clone, PartialEq)]
pub struct Author {
    pub initial: String,
    pub name: String,
    pub kind: AuthorKind,
    /// Explicit avatar tint, when the source data pins one (the prototype's
    /// per-person colour map). `None` falls back to the hashed palette.
    pub tint: Option<String>,
}

impl Author {
    pub fn human(initial: &str) -> Self {
        Self {
            initial: initial.into(),
            name: initial.into(),
            kind: AuthorKind::Human,
            tint: None,
        }
    }
    pub fn agent(initial: &str) -> Self {
        Self {
            initial: initial.into(),
            name: initial.into(),
            kind: AuthorKind::Agent,
            tint: None,
        }
    }
    pub fn human_named(initial: &str, name: &str) -> Self {
        Self {
            initial: initial.into(),
            name: name.into(),
            kind: AuthorKind::Human,
            tint: None,
        }
    }
    pub fn agent_named(initial: &str, name: &str) -> Self {
        Self {
            initial: initial.into(),
            name: name.into(),
            kind: AuthorKind::Agent,
            tint: None,
        }
    }

    /// A human author with a name and an explicit avatar colour; initials are
    /// derived from the name (first letter of up to two words).
    pub fn person(name: &str, color: &str) -> Self {
        Self {
            initial: initials(name),
            name: name.into(),
            kind: AuthorKind::Human,
            tint: Some(color.into()),
        }
    }

    /// The avatar tint — the pinned colour if any, else a stable colour hashed
    /// from the initials so callers never pick hex by hand.
    pub fn color(&self) -> String {
        if let Some(t) = &self.tint {
            return t.clone();
        }
        const PALETTE: [&str; 8] = [
            "#3ecf8e", "#a78bfa", "#e0a458", "#e5736b", "#5b9cf0", "#e87fb0", "#46c8c0", "#c0a35e",
        ];
        // FNV-1a over the initials → a deterministic palette index.
        let mut h: u32 = 2166136261;
        for b in self.initial.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        PALETTE[(h as usize) % PALETTE.len()].to_string()
    }
}

/// Initials from a name: first letter of the first two words, uppercased.
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

/// The card-facing view of a decision (the subset `DecisionCard` renders).
#[derive(Clone, PartialEq)]
pub struct Decision {
    pub authors: Vec<Author>,
    pub project: String,
    pub status: Status,
    pub date: String,
    pub title: String,
    pub provenance: String,
    pub authors_label: String,
}

/// A project (codebase/service) with an unread-decision count.
#[derive(Clone, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub unread: u32,
}

impl Project {
    pub fn new(name: &str, unread: u32) -> Self {
        Self {
            id: name.into(),
            name: name.into(),
            unread,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    Shared,
    Personal,
}

/// A group/team — a set of projects that share memory.
#[derive(Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub project_count: u32,
}

/// A cross-project signal: a decision in `from` affects `to`.
#[derive(Clone, PartialEq)]
pub struct Signal {
    pub from: String,
    pub to: String,
    pub risk: Risk,
    pub title: String,
    pub text: String,
}

/// A rejected alternative + why it lost.
#[derive(Clone, PartialEq)]
pub struct Alternative {
    pub option: String,
    pub why_rejected: String,
}

/// Kind of source a decision is anchored to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Transcript,
    Slack,
    Pr,
    Incident,
}

impl SourceKind {
    pub fn icon(self) -> crate::atoms::Glyph {
        match self {
            SourceKind::Transcript => crate::atoms::Glyph::Transcript,
            SourceKind::Slack => crate::atoms::Glyph::Slack,
            SourceKind::Pr => crate::atoms::Glyph::Pr,
            SourceKind::Incident => crate::atoms::Glyph::Warning,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Transcript => "Transcript",
            SourceKind::Slack => "Slack thread",
            SourceKind::Pr => "Pull request",
            SourceKind::Incident => "Incident",
        }
    }
}

/// Openable evidence a decision was derived from.
#[derive(Clone, PartialEq)]
pub struct Source {
    pub kind: SourceKind,
    pub label: String,
    pub when: String,
}

/// One node in a supersession chain.
#[derive(Clone, PartialEq)]
pub struct ChainNode {
    pub title: String,
    pub project: String,
    pub date: String,
    pub status: Status,
    pub current: bool,
}

/// A reference to this decision from another project.
#[derive(Clone, PartialEq)]
pub struct CrossRef {
    pub project: String,
    pub title: String,
    pub why: Option<String>,
}

/// The full decision view — everything the DecisionDetail screen renders.
#[derive(Clone, PartialEq)]
pub struct DecisionDetail {
    pub project: String,
    pub status: Status,
    pub title: String,
    pub summary: String,
    pub context: Option<String>,
    pub consequences: Option<String>,
    pub alternatives: Vec<Alternative>,
    pub sources: Vec<Source>,
    pub chain: Vec<ChainNode>,
    pub related: Vec<CrossRef>,
    pub signals: Vec<Signal>,
    pub authors: Vec<Author>,
    pub captured: String,
    pub provenance: String,
    pub in_expert_context: bool,
}

/// A compact reference to a decision (mini rows: signal sources, search hits).
#[derive(Clone, PartialEq)]
pub struct DecisionRef {
    pub authors: Vec<Author>,
    pub project: String,
    pub status: Status,
    pub title: String,
    pub summary: String,
}

/// The full view of a cross-project signal — what the SignalDetail screen renders.
#[derive(Clone, PartialEq)]
pub struct SignalDetail {
    pub from: String,
    pub to: String,
    pub risk: Risk,
    pub title: String,
    pub consequence: String,
    pub recommended: String,
    pub sources: Vec<DecisionRef>,
}

/// One line of a source conversation; `extracted` marks the passage a decision
/// was derived from.
#[derive(Clone, PartialEq)]
pub struct ConvLine {
    pub speaker: String,
    pub text: String,
    pub extracted: bool,
}

/// The openable source behind a decision — what the SourceViewer screen renders.
#[derive(Clone, PartialEq)]
pub struct SourceView {
    pub kind: SourceKind,
    pub label: String,
    pub when: String,
    pub lines: Vec<ConvLine>,
}

/// Who sent an Expert-chat message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Expert,
}
