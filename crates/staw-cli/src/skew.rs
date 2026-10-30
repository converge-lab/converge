//! Version-skew detection: the CLI is a distributed binary whose wire
//! contract tracks the server's, and pre-1.0 that contract moves. Skew is
//! surfaced, never enforced — loud in `staw init`, one gentle line in
//! the session-start hook, and cached so hooks don't ping the server on
//! every session (once per day is plenty for a version check).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use staw_client::Client;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// This binary's version (the workspace version at build time).
pub const CLI: &str = env!("CARGO_PKG_VERSION");

const DAY: u64 = 24 * 60 * 60;

/// `major.minor`: pre-1.0, the part of a version that moves the wire
/// contract. A patch on either side (a CLI-only fix, say) leaves it.
fn contract(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// The nudge for a known server version — `None` when the contracts
/// match, or when the server's version does not parse.
pub fn nudge(server: &str) -> Option<String> {
    match contract(CLI)?.cmp(&contract(server)?) {
        Ordering::Equal => None,
        Ordering::Less => Some(format!(
            "staw CLI {CLI} is behind server {server} — update the CLI \
             (re-run its install, then `staw init` to repair hooks)"
        )),
        Ordering::Greater => Some(format!(
            "staw CLI {CLI} is ahead of server {server} — update the server, \
             or match it with `staw update --version v{server}`"
        )),
    }
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
    Some(crate::state::dir()?.join("version.json"))
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
        let (major, minor) = contract(CLI).unwrap();
        assert!(nudge(&format!("{major}.{minor}.999")).is_none());
        assert!(nudge("dev").is_none());

        let behind = nudge("9.9.9").unwrap();
        assert!(behind.contains(CLI) && behind.contains("behind server 9.9.9"));
        let ahead = nudge("0.0.1").unwrap();
        assert!(ahead.contains("ahead of server 0.0.1") && ahead.contains("--version v0.0.1"));
    }
}
