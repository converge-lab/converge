//! Keeping the evidence backlog near zero.
//!
//! A decision cites the conversation that produced it, and the pre-tool
//! hook has seconds to make sure that conversation is on the server. It
//! can only do that if there is little left to send, so this runs on
//! every prompt instead: a bounded pass, oldest turns first, in a
//! process detached from the prompt that spawned it.
//!
//! Two rules keep the record readable. The watermark only moves forward
//! and nothing is ever sent below it, so what a hook passes over is
//! passed over for good rather than arriving later out of sequence. And
//! one writer at a time, held by a lock file per transcript, so a drain
//! and a session's own sync cannot interleave on the same watermark.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use converge_client::ProjectId;

use crate::config::Config;
use crate::harness::{Kind, Transcript};
use crate::marker::{self, State};

/// Turns per request: one comfortable message batch.
const BATCH: usize = 50;
/// Turns per run. A long backlog is drained over several prompts rather
/// than one long process, so a session that ends early loses one pass,
/// not the work.
const MAX_PER_RUN: usize = 500;
/// A lock older than this belonged to a drain that was killed.
const LOCK_STALE: Duration = Duration::from_secs(5 * 60);

/// Hold a transcript's drain lock for as long as this lives.
pub(crate) struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Lock {
    /// Take it, waiting up to `wait` for whoever holds it to finish —
    /// a background pass releases between batches, so a short wait is
    /// usually enough for the hook to get its turn.
    pub(crate) fn take_within(key: &str, wait: Duration) -> Option<Self> {
        let deadline = SystemTime::now() + wait;
        loop {
            if let Some(lock) = Self::take(key) {
                return Some(lock);
            }
            if SystemTime::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }

    /// Take it, or `None` when another drain holds it. A lock left
    /// behind by a killed process is reclaimed once it goes stale.
    fn take(key: &str) -> Option<Self> {
        let path = crate::watermark::state_dir()?.join(format!("drain-{}.lock", digest(key)));
        let _ = path.parent().map(std::fs::create_dir_all);
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Some(Lock(path)),
                Err(_) => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if !stale {
                        return None;
                    }
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        None
    }
}

/// Is a drain running for this transcript right now?
pub fn locked(key: &str) -> bool {
    let Some(path) =
        crate::watermark::state_dir().map(|d| d.join(format!("drain-{}.lock", digest(key))))
    else {
        return false;
    };
    std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age <= LOCK_STALE)
}

/// A short, stable file name for a transcript key, which may be a path.
fn digest(key: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(key.as_bytes()))[..16].to_owned()
}

/// What the poll hook spawns: one pass over the whole backlog, bounded
/// by [`MAX_PER_RUN`]. Quiet throughout — nothing here is worth
/// interrupting a session over.
pub async fn run(kind: Kind, cwd: &Path, transcript: &str) -> Result<()> {
    let at = match kind {
        // opencode keeps conversations in SQLite and names them by
        // session id; everyone else hands over a path.
        Kind::Opencode => Transcript::Session(transcript.to_owned()),
        _ => Transcript::File(PathBuf::from(transcript)),
    };
    let Some(lock) = Lock::take(&at.key()) else {
        return Ok(());
    };
    pass(kind, cwd, &at, MAX_PER_RUN, lock).await?;
    Ok(())
}

/// Send what the server does not have of `at`, oldest first, at most
/// `max` turns, and answer with the ids that landed and how many turns
/// are still waiting. The caller brings the lock, so there is exactly
/// one writer per transcript and the order turns are sent in is the
/// order the conversation is read back in.
pub(crate) async fn pass(
    kind: Kind,
    cwd: &Path,
    at: &Transcript,
    max: usize,
    _lock: Lock,
) -> Result<(Vec<converge_client::MessageId>, usize)> {
    let Ok(State::Bound { project, .. }) = marker::find(cwd) else {
        return Ok((Vec::new(), 0));
    };
    let harness = kind.harness();
    let at = at.clone();
    let key = at.key();
    let parsed = harness.transcript(&at)?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok((Vec::new(), 0));
    };
    let by_id = !parsed.turns.is_empty() && parsed.turns.iter().all(|t| t.id.is_some());

    let mut marks = crate::watermark::Watermarks::load()?;
    if marks.pending(&key, &parsed.turns).is_empty() {
        return Ok((Vec::new(), 0));
    }
    let client = Config::load()?.client()?;
    let session = client
        .session_ensure(&converge_client::NewSession {
            project_id: ProjectId::from(project.ulid()),
            kind: converge_client::SessionKind::Transcript,
            external,
            title: crate::transcript::title(&parsed.turns, &format!("{} session", harness.label())),
        })
        .await
        .context("open the session to drain into")?;

    let mut landed: Vec<converge_client::MessageId> = Vec::new();
    let mut sent = 0usize;
    while sent < max {
        let pending = marks.pending(&key, &parsed.turns);
        if pending.is_empty() {
            break;
        }
        // Oldest first, always: the order turns are sent in is the order
        // the conversation is read back in.
        let batch: Vec<_> = pending.iter().take(BATCH).copied().collect();
        let messages: Vec<_> = batch
            .iter()
            .map(|t| converge_client::NewMessage {
                speaker: t.speaker.clone(),
                body: t.body.clone(),
                sent_at: t.sent_at,
            })
            .collect();
        landed.extend(
            client
                .message_add(session, &messages)
                .await
                .context("send a batch of turns")?,
        );
        sent += batch.len();
        // Mark exactly what is now on the server: the ids when the
        // harness gives them, otherwise the prefix that has been sent.
        if by_id {
            let turns: Vec<crate::transcript::Turn> = batch.iter().map(|t| (*t).clone()).collect();
            marks.done(&key, &turns);
        } else {
            let done = parsed.turns.len() - pending.len() + batch.len();
            marks.done(&key, &parsed.turns[..done]);
        }
        marks.save()?;
    }
    let left = marks.pending(&key, &parsed.turns).len();
    Ok((landed, left))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-drain-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        dir
    }

    #[test]
    fn one_drain_holds_the_lock_and_a_stale_one_is_reclaimed() {
        let _env = crate::watermark::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp_state();
        let key = "/repo/transcript.jsonl";

        let held = Lock::take(key).expect("the first drain takes it");
        assert!(locked(key));
        assert!(Lock::take(key).is_none(), "a second drain stands down");

        // Dropping it lets the next one through.
        drop(held);
        assert!(!locked(key));
        let held = Lock::take(key).expect("the lock was released");

        // A lock left by a killed process goes stale and is reclaimed.
        let path = crate::watermark::state_dir()
            .unwrap()
            .join(format!("drain-{}.lock", digest(key)));
        let old = SystemTime::now() - LOCK_STALE - Duration::from_secs(60);
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!locked(key), "a stale lock holds nobody");
        std::mem::forget(held);
        assert!(Lock::take(key).is_some(), "stale means reclaimable");

        unsafe { std::env::remove_var("XDG_STATE_HOME") };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_transcript_key_becomes_a_short_stable_name() {
        let a = digest("/home/x/.claude/projects/p/abc.jsonl");
        assert_eq!(a.len(), 16);
        assert_eq!(a, digest("/home/x/.claude/projects/p/abc.jsonl"));
        assert_ne!(a, digest("session:abc"));
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
