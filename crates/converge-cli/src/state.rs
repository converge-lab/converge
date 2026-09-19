//! Where this machine keeps what it has done and seen.

use std::path::PathBuf;

/// `$XDG_STATE_HOME/converge` (default `~/.local/state/converge`).
/// State, not cache — nothing here is safe to delete without a
/// consequence, which is why the index cache lives elsewhere.
///
/// What a session has sent is *not* here: a turn carries its position
/// and the server answers where to resume, so the archive needs no
/// local record and cannot disagree with the server about one.
pub(crate) fn dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.local/state")))
        .ok()?;
    Some(PathBuf::from(base).join("converge"))
}

/// Tests that point the state directory at a temporary one mutate a
/// process-wide environment variable, so they take turns rather than
/// reading each other's half-set world.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
