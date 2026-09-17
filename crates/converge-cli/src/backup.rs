//! Snapshots of the files the integration writes, taken before every
//! write, so an update — or an init — can be undone file for file. One
//! directory per occasion under `$XDG_STATE_HOME/converge/backups/`,
//! with a manifest naming each file's original path and whether it
//! existed at all; the last ten occasions are kept.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const KEEP: usize = 10;

/// One kept file: where it lives, what it is called here, and whether
/// there was anything to keep — a file the write is about to create is
/// recorded as absent, so a restore removes it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    copy: String,
    existed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    reason: String,
    version: String,
    at: u64,
    entries: Vec<Entry>,
}

/// An occasion's snapshot, filled as files are kept and written out at
/// the end. Nothing kept means nothing on disk.
#[derive(Debug)]
pub struct Snapshot {
    dir: PathBuf,
    reason: String,
    entries: Vec<Entry>,
}

impl Snapshot {
    /// Start one, named for the occasion: `init`, `update-0.1.24-0.1.25`,
    /// `rollback`. `None` when there is no state dir to keep it in.
    pub fn begin(reason: &str) -> Option<Self> {
        let root = root()?;
        let at = crate::poll::now();
        Some(Self {
            dir: root.join(format!("{at:010}-{reason}")),
            reason: reason.to_owned(),
            entries: Vec::new(),
        })
    }

    /// Copy `path` aside before it is written, once per path.
    pub fn keep(&mut self, path: &Path) -> Result<()> {
        if self.entries.iter().any(|e| e.path == path) {
            return Ok(());
        }
        let copy = format!(
            "{}.{}",
            self.entries.len(),
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into())
        );
        let existed = path.exists();
        if existed {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("create {}", self.dir.display()))?;
            std::fs::copy(path, self.dir.join(&copy))
                .with_context(|| format!("keep a copy of {}", path.display()))?;
        }
        self.entries.push(Entry {
            path: path.to_path_buf(),
            copy,
            existed,
        });
        Ok(())
    }

    /// Write the manifest and prune old occasions. Returns where the
    /// snapshot is, or `None` when nothing was kept.
    pub fn finish(self) -> Result<Option<PathBuf>> {
        if self.entries.is_empty() {
            return Ok(None);
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create {}", self.dir.display()))?;
        let manifest = Manifest {
            reason: self.reason,
            version: crate::skew::CLI.to_owned(),
            at: crate::poll::now(),
            entries: self.entries,
        };
        std::fs::write(
            self.dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?,
        )
        .with_context(|| format!("write {}", self.dir.display()))?;
        if let Some(parent) = self.dir.parent() {
            prune(parent, KEEP);
        }
        Ok(Some(self.dir))
    }
}

/// The newest snapshot whose reason starts with `prefix`.
pub fn latest(prefix: &str) -> Option<PathBuf> {
    let root = root()?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.join("manifest.json").exists()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.split_once('-'))
                    .is_some_and(|(_, reason)| reason.starts_with(prefix))
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

/// Put every file in a snapshot back as it was: copied over, or removed
/// when it had not existed. Returns the paths actually written or
/// removed — a file absent then and absent now is left out.
pub fn restore(dir: &Path) -> Result<Vec<PathBuf>> {
    let text = std::fs::read_to_string(dir.join("manifest.json"))
        .with_context(|| format!("read the manifest in {}", dir.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("parse the manifest in {}", dir.display()))?;
    let mut touched = Vec::new();
    for entry in manifest.entries {
        if entry.existed {
            if let Some(parent) = entry.path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(dir.join(&entry.copy), &entry.path)
                .with_context(|| format!("restore {}", entry.path.display()))?;
        } else if entry.path.exists() {
            std::fs::remove_file(&entry.path)
                .with_context(|| format!("remove {}", entry.path.display()))?;
        } else {
            continue;
        }
        touched.push(entry.path);
    }
    Ok(touched)
}

fn root() -> Option<PathBuf> {
    Some(crate::watermark::state_dir()?.join("backups"))
}

/// Keep the newest `keep` occasions; the directory names sort by time.
fn prune(root: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    while dirs.len() > keep {
        let _ = std::fs::remove_dir_all(dirs.remove(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-backup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keeps_what_existed_and_records_what_did_not() {
        let home = temp();
        let hooks = home.join("hooks.json");
        let plugin = home.join("plugin").join("converge.js");
        std::fs::write(&hooks, "old").unwrap();

        let mut snapshot = Snapshot {
            dir: home.join("backups").join("0000000001-update"),
            reason: "update".into(),
            entries: Vec::new(),
        };
        snapshot.keep(&hooks).unwrap();
        snapshot.keep(&plugin).unwrap();
        snapshot.keep(&hooks).unwrap(); // once per path
        let dir = snapshot.finish().unwrap().expect("something was kept");

        // The write happens.
        std::fs::write(&hooks, "new").unwrap();
        std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        std::fs::write(&plugin, "shim").unwrap();

        // And is undone: the old bytes are back, the created file is gone.
        let touched = restore(&dir).unwrap();
        assert_eq!(touched, vec![hooks.clone(), plugin.clone()]);
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), "old");
        assert!(!plugin.exists());

        // Nothing kept, nothing written.
        let empty = Snapshot {
            dir: home.join("backups").join("0000000002-init"),
            reason: "init".into(),
            entries: Vec::new(),
        };
        assert_eq!(empty.finish().unwrap(), None);
        assert!(!home.join("backups").join("0000000002-init").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn prune_keeps_the_newest() {
        let root = temp();
        for n in 1..=4 {
            std::fs::create_dir_all(root.join(format!("{n:010}-init"))).unwrap();
        }
        prune(&root, 2);
        let mut left: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["0000000003-init", "0000000004-init"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
