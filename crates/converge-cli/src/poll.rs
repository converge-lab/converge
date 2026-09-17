//! The per-prompt poll's local state: when each session last polled,
//! whether that poll failed, and how much it has been handed. These are
//! the gates that keep a hook on every prompt cheap — and the tier
//! floor. A JSON map under the state dir, pruned of sessions idle a week.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use converge_client::Tier;
use serde::{Deserialize, Serialize};

/// Between polls of one session, after a poll that worked. Prompts come
/// faster than signals; one request every twenty seconds is the ceiling
/// a chatty session puts on the server.
pub const INTERVAL_SECS: u64 = 20;
/// After a poll that failed. A server that is down stays down for a
/// while, and a prompt must not pay the timeout for it every time.
pub const BACKOFF_SECS: u64 = 120;
/// A session idle this long is forgotten.
const FORGET_SECS: u64 = 7 * 24 * 3600;

/// One session's poll history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// Unix seconds of the last poll.
    pub at: u64,
    pub failed: bool,
    /// Signals shown so far.
    pub delivered: u32,
}

/// Is a poll due for a session in this state?
pub fn due(stamp: Option<&Stamp>, now: u64) -> bool {
    stamp.is_none_or(|s| {
        let wait = if s.failed {
            BACKOFF_SECS
        } else {
            INTERVAL_SECS
        };
        now.saturating_sub(s.at) >= wait
    })
}

/// The lowest tier a session is still shown mid-conversation. Its first
/// delivery passes everything, so a watch-tier note about the decision
/// it just recorded gets through once; after that only coordinate and
/// conflict interrupt a prompt — watch is for the session-start listing.
pub fn floor(stamp: Option<&Stamp>) -> Tier {
    if stamp.is_none_or(|s| s.delivered == 0) {
        Tier::Watch
    } else {
        Tier::Coordinate
    }
}

/// Every session this machine has polled for.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Stamps {
    sessions: HashMap<String, Stamp>,
}

impl Stamps {
    /// Unreadable or absent is empty: the cost is one extra poll.
    pub fn load() -> Self {
        path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, session: &str) -> Option<&Stamp> {
        self.sessions.get(session)
    }

    /// What just happened to a session's poll. Idle sessions leave here,
    /// so the map stays as big as a week of work.
    pub fn record(&mut self, session: &str, now: u64, failed: bool, delivered: usize) {
        let stamp = self.sessions.entry(session.to_owned()).or_default();
        stamp.at = now;
        stamp.failed = failed;
        stamp.delivered = stamp
            .delivered
            .saturating_add(u32::try_from(delivered).unwrap_or(u32::MAX));
        self.sessions
            .retain(|_, s| now.saturating_sub(s.at) < FORGET_SECS);
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        // Atomic: a hook killed mid-write must leave the previous map
        // intact, never a truncated file.
        let text = serde_json::to_string_pretty(&self.sessions)?;
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }
}

fn path() -> Option<PathBuf> {
    Some(crate::watermark::state_dir()?.join("poll.json"))
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_hold_between_polls_and_longer_after_a_failure() {
        assert!(due(None, 0));
        let ok = Stamp {
            at: 1_000,
            failed: false,
            delivered: 0,
        };
        assert!(!due(Some(&ok), 1_000 + INTERVAL_SECS - 1));
        assert!(due(Some(&ok), 1_000 + INTERVAL_SECS));
        let failed = Stamp {
            failed: true,
            ..ok.clone()
        };
        assert!(!due(Some(&failed), 1_000 + INTERVAL_SECS));
        assert!(due(Some(&failed), 1_000 + BACKOFF_SECS));
        // A clock that went backwards never locks a session out.
        assert!(!due(Some(&ok), 900));
        assert!(due(Some(&ok), 900 + INTERVAL_SECS + 100));
    }

    #[test]
    fn first_delivery_passes_watch_then_the_floor_rises() {
        assert_eq!(floor(None), Tier::Watch);
        let mut stamps = Stamps::default();
        stamps.record("s", 10, false, 0);
        assert_eq!(floor(stamps.get("s")), Tier::Watch);
        stamps.record("s", 40, false, 2);
        assert_eq!(floor(stamps.get("s")), Tier::Coordinate);
        assert_eq!(stamps.get("s").map(|s| s.delivered), Some(2));
        // A later failure keeps what was delivered.
        stamps.record("s", 70, true, 0);
        assert_eq!(
            stamps.get("s"),
            Some(&Stamp {
                at: 70,
                failed: true,
                delivered: 2
            })
        );
    }

    #[test]
    fn idle_sessions_are_forgotten() {
        let mut stamps = Stamps::default();
        stamps.record("old", 0, false, 1);
        stamps.record("new", FORGET_SECS + 1, false, 0);
        assert!(stamps.get("old").is_none());
        assert!(stamps.get("new").is_some());
        // The file shape is the bare map, so a hand-read is one level.
        let text = serde_json::to_string(&stamps).unwrap();
        assert!(text.starts_with("{\"new\":"), "{text}");
    }
}
