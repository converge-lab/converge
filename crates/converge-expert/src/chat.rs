//! Placeholder contract for a grounded expert conversation.

use serde::{Deserialize, Serialize};

/// Input of the future `chat` operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request;

/// Output of the future `chat` operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response;
