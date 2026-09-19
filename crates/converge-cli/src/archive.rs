//! Keeping the conversation, after the decision.
//!
//! A decision carries the turns it cites, so nothing a decision needs
//! waits on this. What is left is the rest of the transcript, kept for
//! whoever reads the conversation later, and whether it is kept at all
//! is the project's call, not this machine's.
//!
//! No local record of what has been sent: every turn names its
//! position, the server writes one row per position and answers where
//! to resume, so two senders cannot disagree and a lost state file
//! cannot cost or duplicate a conversation.

use std::path::Path;

use anyhow::{Context, Result};
use converge_client::ProjectId;

use crate::config::Config;
use crate::harness::{Kind, Transcript};
use crate::marker::{self, State};

/// Turns per request: one comfortable batch.
const BATCH: usize = 50;
/// When there is something new to send, a few already-sent turns go
/// with it. Re-sending is free — the server keeps one row per position
/// — and it is how a turn a harness rewrote behind the resume point
/// gets recorded at all.
const OVERLAP: usize = 5;

/// What this pass found and did: how many turns it sent, and how many
/// it left for the next one.
pub(crate) struct Sent {
    pub sent: usize,
    pub left: usize,
}

/// Send what the server does not have of `at`, oldest first, at most
/// `max` turns. Quiet about everything it cannot do: an unbound tree,
/// a project that keeps no archive, a transcript with nothing new.
pub(crate) async fn pass(kind: Kind, cwd: &Path, at: &Transcript, max: usize) -> Result<Sent> {
    let nothing = Sent { sent: 0, left: 0 };
    let Ok(State::Bound { project, .. }) = marker::find(cwd) else {
        return Ok(nothing);
    };
    let harness = kind.harness();
    let parsed = harness.transcript(at)?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok(nothing);
    };
    if parsed.turns.is_empty() {
        return Ok(nothing);
    }
    let client = Config::load()?.client()?;
    let opened = client
        .session_ensure(&converge_client::NewSession {
            project_id: ProjectId::from(project.ulid()),
            kind: converge_client::SessionKind::Transcript,
            external,
            title: crate::transcript::title(&parsed.turns, &format!("{} session", harness.label())),
        })
        .await
        .context("open the session to record into")?;
    // The project keeps only what its decisions cite. Those turns ride
    // on the decision itself, so there is nothing to do here.
    if !opened.archive_transcripts {
        return Ok(nothing);
    }
    let resume = usize::try_from(opened.next_ordinal).unwrap_or(0);
    if resume >= parsed.turns.len() {
        return Ok(nothing);
    }
    let from = resume.saturating_sub(OVERLAP);

    let mut sent = 0usize;
    for batch in parsed.turns[from..].chunks(BATCH) {
        if sent >= max {
            break;
        }
        let messages: Vec<_> = batch
            .iter()
            .enumerate()
            .map(|(i, turn)| converge_client::NewMessage {
                speaker: turn.speaker.clone(),
                body: turn.body.clone(),
                sent_at: turn.sent_at,
                // Where the turn sits in the transcript: the server
                // writes it once, whoever sends it.
                ordinal: i32::try_from(from + sent + i).ok(),
            })
            .collect();
        client
            .message_add(opened.id, &messages)
            .await
            .context("send a batch of turns")?;
        sent += batch.len();
    }
    Ok(Sent {
        sent,
        left: parsed.turns.len().saturating_sub(from + sent),
    })
}
