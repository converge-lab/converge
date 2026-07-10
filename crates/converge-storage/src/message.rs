//! The message — one line of a session's stream, and the unit decisions
//! anchor to as evidence.
//!
//! Streams are **append-only**: no edit or delete operation exists,
//! because evidence you can rewrite isn't evidence. Corrections happen at
//! the decision layer (supersession), never by touching history.

use std::future::Future;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{MessageId, SessionId};
use crate::{Pagination, StoreError};

/// A message, as stored and served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    /// Position within the session (0-based, dense per batch, gap-free
    /// ordering not guaranteed across batches — only the order is).
    pub seq: i32,
    /// Display name from the source system — Slack authors and PR
    /// reviewers aren't converge users, so this is a string, not an id.
    pub speaker: String,
    pub body: String,
    /// When it was said in the source system — an external *fact* carried
    /// by importers; absent for live-recorded messages.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
    /// When converge learned of it (server-assigned).
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

/// A message to append (the server assigns `seq` and `captured_at`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMessage {
    pub speaker: String,
    pub body: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
}

/// Storage operations on messages.
pub trait Messages {
    /// Append a batch to a session, in order, atomically; returns the new
    /// ids. Appends to one session are serialized (concurrent batches
    /// can't interleave or collide on `seq`). An unknown session is
    /// `NotFound`.
    fn message_add(
        &self,
        session: SessionId,
        new: Vec<NewMessage>,
    ) -> impl Future<Output = Result<Vec<MessageId>, StoreError>> + Send;

    /// A session's stream in conversation order — **oldest first**, the
    /// one list in the system that reads forward. The cursor returns
    /// messages strictly *after* it.
    fn message_list(
        &self,
        session: SessionId,
        page: Pagination<MessageId>,
    ) -> impl Future<Output = Result<Vec<Message>, StoreError>> + Send;
}
