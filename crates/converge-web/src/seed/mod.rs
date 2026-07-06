//! The embedded fixture dataset: relational seed rows, the schema
//! constraints as code (`validate`), the rows→read-model assembly
//! (`assemble`), and the shapes it produces (`wire`).
//!
//! This is an app-local *fixture* format, not an API contract — the real
//! wire contract is `converge-client`\'s typed surface, and the HTTP
//! `ApiSource` converts client responses into this read-model in code.
//! Fixture ids are human-readable strings ("status-field"), deliberately
//! not ULIDs. Everything not yet backed by the real server (signals,
//! unread, extras, expert context) lives under [`wire::mock`] — moving a
//! type out of there is the signal that it gained a real endpoint.

pub mod assemble;
pub mod enums;
pub mod rows;
// Validation guards the embedded path's debug builds; with `api` on that
// path is dead code by design.
#[cfg_attr(feature = "api", allow(dead_code))]
pub mod validate;
pub mod wire;

pub use assemble::{Assembled, assemble};
pub use enums::{GroupKind, Risk, SourceKind, Status};
pub use rows::{EMBEDDED, Seed};
pub use validate::validate;
