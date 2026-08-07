//! The Converge server — the product's HTTP surface over the storage seam.
//!
//! The versioned web API lives under `/api/v1`; the MCP endpoint (`/mcp`,
//! unversioned, stateless) lands in a later slice. Everything is written
//! against the `converge_storage` traits, never a concrete backend — the
//! binary picks the backend (PostgreSQL) at the edge.
//!
//! `config` and `telemetry` are the composition-root pieces the bundled
//! binary shares with embedders: an overlay (a hosted deployment
//! composing [`app`] with extra routes) loads the same layered
//! configuration and logging instead of reinventing them.

pub mod auth;
pub mod config;
pub mod expert;
pub mod http;
pub mod mcp;
pub mod oauth;
pub mod oidc;
pub mod telemetry;

pub use expert::{Backfill, Expert};
pub use http::app;
