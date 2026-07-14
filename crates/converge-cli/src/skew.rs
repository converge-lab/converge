//! Version-skew detection: the CLI is a distributed binary whose wire
//! contract tracks the server's, and pre-1.0 that contract moves. Skew is
//! surfaced, never enforced — loud in `converge init`, one gentle line in
//! the session-start hook, and cached so hooks don't ping the server on
//! every session (once per day is plenty for a version check).

use anyhow::Result;
use converge_client::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// This binary's version (the workspace version at build time).
pub const CLI: &str = env!("CARGO_PKG_VERSION");

const DAY: u64 = 24 * 60 * 60;

/// The nudge for a known server version — `None` when in sync.
pub fn nudge(server: &str) -> Option<String> {
    (server != CLI).then(|| {
        format!(
            "converge CLI {CLI} ≠ server {server} — update the CLI \
             (re-run its install, then `converge init` to repair hooks)"
        )
    })
}

/// Fresh check against the server (init's loud path).
pub async fn check(client: &Client) -> Option<String> {
    nudge(&client.version().await.ok()?)
}

#[derive(Serialize, Deserialize)]
struct Cached {
    checked_at: u64,
    server: String,
}

/// Daily-cached check (the hooks' gentle path): at most one healthz call
/// per day per machine; any failure quietly reports "no nudge".
pub async fn check_cached(client: &Client) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if let Some(cached) = load()
        && now.saturating_sub(cached.checked_at) < DAY
    {
        return nudge(&cached.server);
    }
    let server = client.version().await.ok()?;
    let _ = save(&Cached {
        checked_at: now,
        server: server.clone(),
    });
    nudge(&server)
}

fn path() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.local/state")))
        .ok()?;
    Some(PathBuf::from(base).join("converge/version.json"))
}

fn load() -> Option<Cached> {
    serde_json::from_str(&std::fs::read_to_string(path()?).ok()?).ok()
}

fn save(cached: &Cached) -> Result<()> {
    let Some(path) = path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string(cached)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudges_only_on_skew() {
        assert!(nudge(CLI).is_none());
        let msg = nudge("9.9.9").unwrap();
        assert!(msg.contains(CLI) && msg.contains("9.9.9"));
    }
}
