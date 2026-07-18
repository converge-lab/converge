//! The Converge expert — the model boundary of the product.
//!
//! [`expert::Expert`] is a configured model actor with one method per
//! operation; signal discovery is the first real one, over a multi-provider
//! genai transport. Related-memory selection and chat remain data-contract
//! placeholders.

pub mod chat;
pub mod clients;
pub mod expert;
pub mod related;
pub mod signals;

pub use expert::{Config, Expert, Reasoning};
