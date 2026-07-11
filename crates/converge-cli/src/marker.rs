//! The committed binding state: a `.converge` file at the repository
//! root. Three states, no silent default (the rule the POC validated):
//!
//! | `.converge`          | meaning                                   |
//! |----------------------|-------------------------------------------|
//! | `project_id = "…"`   | **bound** — sessions resolve here          |
//! | `disable = true`     | **opted out** — integration stays quiet    |
//! | absent               | **unbound** — suggest on next session      |
//!
//! Committing the file is the point: every teammate (and their agents)
//! resolves identically. The id is a ULID — nothing resolves by name.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use converge_client::ProjectId;
use serde::Deserialize;

pub const FILE: &str = ".converge";

/// What the working tree says about converge.
#[derive(Debug)]
pub enum State {
    /// `project_id = "…"` — bound to this project.
    Bound { path: PathBuf, project: ProjectId },
    /// `disable = true` — the integration is off here, on purpose.
    Disabled { path: PathBuf },
    /// No marker anywhere up the tree.
    Unbound,
}

#[derive(Deserialize)]
struct Marker {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    disable: bool,
}

/// Resolve the binding state from `start` upward (nearest marker wins).
/// `disable` beats `project_id` when both appear; a marker with neither
/// is a broken state and fails loudly — re-run `converge project init`.
pub fn find(start: &Path) -> Result<State> {
    for dir in start.ancestors() {
        let path = dir.join(FILE);
        if !path.exists() {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let marker: Marker =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if marker.disable {
            return Ok(State::Disabled { path });
        }
        let Some(id) = marker.project_id else {
            bail!(
                "{} has neither `project_id` nor `disable = true` — \
                 re-run `converge project init`",
                path.display()
            );
        };
        let project = id
            .parse()
            .with_context(|| format!("{}: `{id}` is not a project id", path.display()))?;
        return Ok(State::Bound { path, project });
    }
    Ok(State::Unbound)
}

/// Where a new marker belongs: the git toplevel when there is one (so
/// monorepo-subdirectory sessions resolve consistently), else `dir`.
pub fn root(dir: &Path) -> PathBuf {
    Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| dir.to_path_buf())
}

/// Write the bound state (the project name rides as a comment — the id
/// is the only source of truth; names drift).
pub fn write_bound(dir: &Path, project: ProjectId, name: &str) -> Result<PathBuf> {
    let path = dir.join(FILE);
    let text = format!(
        "# Binds this repository to the converge project \"{name}\".\n\
         # Committed on purpose: every teammate's sessions resolve here.\n\
         project_id = \"{project}\"\n"
    );
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Write the opted-out state.
pub fn write_disabled(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(FILE);
    let text = "# Converge is off for this repository, on purpose.\n\
                # Remove this file (or re-run `converge project init --rebind`) to re-enable.\n\
                disable = true\n";
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        dir
    }

    #[test]
    fn three_states_walk_up() {
        let root = temp();
        let nested = root.join("a/b");

        assert!(matches!(find(&nested).unwrap(), State::Unbound));

        let id = ProjectId::new();
        write_bound(&root, id, "demo").unwrap();
        match find(&nested).unwrap() {
            State::Bound { path, project } => {
                assert_eq!(project, id);
                assert_eq!(path, root.join(FILE));
            }
            other => panic!("expected bound, got {other:?}"),
        }

        write_disabled(&root).unwrap();
        assert!(matches!(find(&nested).unwrap(), State::Disabled { .. }));

        // The nearest marker wins over an ancestor's.
        write_bound(&nested, id, "inner").unwrap();
        assert!(matches!(find(&nested).unwrap(), State::Bound { .. }));

        // Neither key → loud error, not a guess.
        std::fs::write(root.join("a/b").join(FILE), "# empty\n").unwrap();
        assert!(find(&nested).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
