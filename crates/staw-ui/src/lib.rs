//! staw-ui — Staw's component library.
//!
//! Pages are composed from these components; `staw-gallery` renders every
//! atom and molecule in isolation (the living catalog).

pub mod atoms;
pub mod domain;
pub mod layout;
pub mod md;
pub mod molecules;

pub use atoms::Glyph;
pub use domain::{
    Alternative, Author, AuthorKind, ChainNode, ChatRole, ConvLine, CrossRef, Decision,
    DecisionDetail, DecisionRef, Group, GroupKind, Project, Risk, Signal, SignalDetail, Source,
    SourceKind, SourceView, Status, Tone,
};
pub use md::Markdown;
