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
use crate::{Pagination, Scope, StoreError};

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
    /// Where this turn sits in the conversation it came from — the
    /// index in a transcript. Two sends of the same position and the
    /// same body are one message, and reads order by it, so a turn
    /// that arrives late still reads in its place. The same position
    /// with a different body is a different turn, recorded without a
    /// position. Absent for writers that cannot number their turns;
    /// those keep arrival order.
    #[serde(default)]
    pub ordinal: Option<i32>,
}

/// Storage operations on messages.
pub trait Messages {
    /// Append a batch to a session, in order, atomically; returns their
    /// ids — including for turns already recorded at the same
    /// `ordinal`, which are not written twice, so a caller can cite
    /// what it sent whether or not it got there first. Appends to one
    /// session are serialized (concurrent batches can't interleave or
    /// collide on `seq`). An unknown session is `NotFound`.
    fn message_add(
        &self,
        scope: Scope,
        session: SessionId,
        new: Vec<NewMessage>,
    ) -> impl Future<Output = Result<Vec<MessageId>, StoreError>> + Send;

    /// Where a sender resumes: the first position this session holds no
    /// turn for. The highest `ordinal` plus one, or — for a session
    /// recorded before turns carried a position — how many turns it
    /// holds, which for a CLI-written session counted the same way. So
    /// a client needs no durable record of what it has sent. An unknown
    /// session is `NotFound`.
    fn message_next_ordinal(
        &self,
        scope: Scope,
        session: SessionId,
    ) -> impl Future<Output = Result<i32, StoreError>> + Send;

    /// A session's stream in conversation order — **oldest first**, the
    /// one list in the system that reads forward. The cursor returns
    /// messages strictly *after* it.
    fn message_list(
        &self,
        scope: Scope,
        session: SessionId,
        page: Pagination<MessageId>,
    ) -> impl Future<Output = Result<Vec<Message>, StoreError>> + Send;
}
