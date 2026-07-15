//! Placeholder contract for selecting related decision memory.

use serde::{Deserialize, Serialize};

/// Input of the future `related` operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request;

/// Output of the future `related` operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response;
