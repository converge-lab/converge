//! Per-transcript sync watermarks — what of each transcript has already
//! been pushed. A JSON map under `$XDG_STATE_HOME/converge/sync.json`
//! (default `~/.local/state`), keyed by the transcript's sync key.
//!
//! Two kinds of mark. A **count** of turns, for append-only transcripts
//! (Claude Code, Codex): the file only grows, so "the first N are done"
//! is exact. A **set of message ids**, for transcripts that are edited
//! in place (opencode lets a user revert turns): after a revert the
//! count is back where it was and a count-based mark would never send
//! the replacement — ids say precisely which messages went.
//!
//! Machine-local state, not committed: it records what *this* machine
//! has sent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::transcript::Turn;

/// One transcript's mark. `Count` is the original on-disk form, so
/// existing files read unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Mark {
    Count(usize),
    Ids(BTreeSet<String>),
}

/// The whole map, loaded and saved together.
#[derive(Default)]
pub struct Watermarks {
    synced: BTreeMap<String, Mark>,
}

impl Watermarks {
    pub fn load() -> Result<Self> {
        let Some(path) = path() else {
            return Ok(Self::default());
        };
        let synced = match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        Ok(Self { synced })
    }

    /// The turns of `transcript` not yet pushed, in order. By id when
    /// every turn has one; by count otherwise, where a shrunk transcript
    /// (fewer turns than synced) yields nothing rather than a re-send.
    pub fn pending<'a>(&self, transcript: &str, turns: &'a [Turn]) -> Vec<&'a Turn> {
        if turns.iter().all(|t| t.id.is_some()) {
            let done = match self.synced.get(transcript) {
                Some(Mark::Ids(ids)) => Some(ids),
                _ => None,
            };
            turns
                .iter()
                .filter(|t| {
                    t.id.as_ref()
                        .is_some_and(|id| !done.is_some_and(|done| done.contains(id)))
                })
                .collect()
        } else {
            let already = match self.synced.get(transcript) {
                Some(Mark::Count(n)) => *n,
                _ => 0,
            };
            turns
                .get(already..)
                .map(|tail| tail.iter().collect())
                .unwrap_or_default()
        }
    }

    /// Record that all of `turns` are now pushed.
    pub fn done(&mut self, transcript: &str, turns: &[Turn]) {
        let mark = if turns.iter().all(|t| t.id.is_some()) {
            let mut ids = match self.synced.get(transcript) {
                Some(Mark::Ids(ids)) => ids.clone(),
                _ => BTreeSet::new(),
            };
            ids.extend(turns.iter().filter_map(|t| t.id.clone()));
            Mark::Ids(ids)
        } else {
            Mark::Count(turns.len())
        };
        self.synced.insert(transcript.to_string(), mark);
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        // Atomic: a sync killed mid-write (a harness timeout) must leave
        // the previous map intact, never a truncated file.
        let text = serde_json::to_string_pretty(&self.synced)?;
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }
}

fn path() -> Option<PathBuf> {
    Some(state_dir()?.join("sync.json"))
}

/// Where this machine keeps what it has done and seen:
/// `$XDG_STATE_HOME/converge` (default `~/.local/state/converge`).
/// State, not cache — nothing here is safe to delete without a
/// consequence, which is why the index cache lives elsewhere.
pub(crate) fn state_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.local/state")))
        .ok()?;
    Some(PathBuf::from(base).join("converge"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: Option<&str>, body: &str) -> Turn {
        Turn {
            speaker: "user".into(),
            body: body.into(),
            sent_at: None,
            id: id.map(str::to_owned),
        }
    }

    #[test]
    fn ids_survive_a_revert_and_counts_stay_append_only() {
        let mut marks = Watermarks::default();

        // opencode: ids. Two turns pushed, one reverted, one replacement.
        let first = [turn(Some("a"), "1"), turn(Some("b"), "2")];
        assert_eq!(marks.pending("s", &first).len(), 2);
        marks.done("s", &first);
        let after_revert = [turn(Some("a"), "1"), turn(Some("c"), "2 again")];
        let pending: Vec<_> = marks
            .pending("s", &after_revert)
            .iter()
            .map(|t| t.body.as_str())
            .collect();
        assert_eq!(pending, ["2 again"], "the replacement must be sent");

        // Claude/Codex: a count. Growth sends the tail; shrink sends nothing.
        let three = [turn(None, "x"), turn(None, "y"), turn(None, "z")];
        marks.done("t", &three[..2]);
        assert_eq!(marks.pending("t", &three).len(), 1);
        assert!(marks.pending("t", &three[..1]).is_empty());

        // The count form on disk still reads.
        let legacy: BTreeMap<String, Mark> = serde_json::from_str(r#"{"t": 2}"#).unwrap();
        assert!(matches!(legacy["t"], Mark::Count(2)));
    }
}
