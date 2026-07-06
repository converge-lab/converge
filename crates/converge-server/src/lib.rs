//! The Converge server — the product's HTTP surface over the storage seam.
//!
//! The versioned web API lives under `/api/v1`; the MCP endpoint (`/mcp`,
//! unversioned, stateless) lands in a later slice. Everything is written
//! against the `converge_storage` traits, never a concrete backend — the
//! binary picks the backend (PostgreSQL) at the edge.

pub mod http;
pub mod mcp;

pub use http::app;
