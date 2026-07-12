//! Per-transcript sync watermarks — how many turns of each transcript
//! have already been pushed. A JSON map (`transcript path → turn count`)
//! under `$XDG_STATE_HOME/converge/sync.json` (default `~/.local/state`).
//!
//! Machine-local state, not committed: it records what *this* machine has
//! sent, so an append-only transcript syncs only its new turns. Counting
//! turns (not bytes) keeps the derived session title stable and survives
//! the transcript growing between syncs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// The whole map, loaded and saved together. Keyed by absolute transcript
/// path; the value is how many conversation turns were synced.
#[derive(Default)]
pub struct Watermarks {
    synced: BTreeMap<String, usize>,
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

    /// How many turns of `transcript` were already synced (0 if never).
    pub fn synced(&self, transcript: &str) -> usize {
        self.synced.get(transcript).copied().unwrap_or(0)
    }

    pub fn set(&mut self, transcript: &str, turns: usize) {
        self.synced.insert(transcript.to_string(), turns);
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&self.synced)?;
        std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.local/state")))
        .ok()?;
    Some(PathBuf::from(base).join("converge/sync.json"))
}
