//! The matcher-group hooks file Claude Code and Codex CLI both use.
//!
//! Same JSON either side of the vendor line: a `hooks` object keyed by
//! event, each event an array of groups, each group an optional `matcher`
//! and a list of `{type: "command", command}` handlers. Claude keeps it
//! among its other settings in `~/.claude/settings.json`; Codex gives it
//! a file of its own at `~/.codex/hooks.json`. Neither cares that the
//! other exists, so the merge below serves both.

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::json;

/// One registration we want present.
pub struct Wanted {
    pub event: &'static str,
    pub matcher: Option<&'static str>,
    pub command: String,
}

impl Wanted {
    /// What identifies this entry as ours across binary moves: everything
    /// from ` hook ` onward, which carries the subcommand and the
    /// `--harness` flag but not the path that may have changed.
    fn tail(&self) -> &str {
        let at = self
            .command
            .find(" hook ")
            .expect("wanted commands contain ` hook `");
        &self.command[at..]
    }
}

/// Merge our entries in, conservatively: existing content is never
/// touched; an entry whose command ends with the same `hook …` tail
/// counts as present (so a moved binary updates in place). Returns which
/// events changed.
pub fn merge(path: &Path, wanted: &[Wanted]) -> Result<Vec<String>> {
    let (mut root, like) = super::json_file::read(path, json!({}))?;
    if !root.is_object() {
        bail!("{} is not a JSON object", path.display());
    }
    if !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }

    let mut changed = Vec::new();
    for want in wanted {
        let tail = want.tail();
        let entries = &mut root["hooks"][want.event];
        if !entries.is_array() {
            *entries = json!([]);
        }
        let list = entries.as_array_mut().expect("just ensured");

        // Present already? Update the command in place (binary may have
        // moved); otherwise append a fresh entry.
        let mut found = false;
        for group in list.iter_mut() {
            let Some(hooks) = group["hooks"].as_array_mut() else {
                continue;
            };
            for hook in hooks.iter_mut() {
                let is_ours = hook["command"].as_str().is_some_and(|c| c.ends_with(tail));
                if is_ours {
                    found = true;
                    if hook["command"].as_str() != Some(want.command.as_str()) {
                        hook["command"] = json!(want.command);
                        changed.push(format!("{} (path updated)", want.event));
                    }
                }
            }
        }
        if !found {
            let mut group = json!({ "hooks": [{ "type": "command", "command": want.command }] });
            if let Some(matcher) = want.matcher {
                group["matcher"] = json!(matcher);
            }
            list.push(group);
            changed.push(want.event.to_string());
        }
    }

    if !changed.is_empty() {
        super::json_file::write(path, &root, &like)?;
    }
    Ok(changed)
}

/// The four registrations the integration needs, for a harness whose
/// hook commands carry `suffix` (empty for Claude Code, whose installed
/// base predates the flag). `exe` is this binary's absolute path.
pub fn wanted(exe: &str, suffix: &str) -> Vec<Wanted> {
    let command = |sub: &str| format!("{exe} hook {sub}{suffix}");
    vec![
        Wanted {
            event: "SessionStart",
            matcher: None,
            command: command("inject"),
        },
        Wanted {
            event: "SessionEnd",
            matcher: None,
            command: command("sync"),
        },
        Wanted {
            event: "PreToolUse",
            matcher: Some("mcp__converge__"),
            command: command("ctx"),
        },
        Wanted {
            event: "PostToolUse",
            matcher: Some("mcp__converge__(project_bind|project_dismiss)"),
            command: command("mark"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn merge_is_conservative_and_idempotent() {
        let dir = std::env::temp_dir().join(format!("cvg-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");

        // Existing user content that must survive untouched.
        std::fs::write(
            &file,
            serde_json::to_string(&json!({
                "permissions": { "allow": ["Bash"] },
                "hooks": {
                    "SessionStart": [
                        { "hooks": [{ "type": "command", "command": "echo hi" }] }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let changed = merge(&file, &wanted("/usr/bin/converge", "")).unwrap();
        assert_eq!(changed.len(), 4, "{changed:?}");

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        // Untouched neighbors.
        assert_eq!(root["permissions"]["allow"][0], "Bash");
        assert_eq!(
            root["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "echo hi"
        );
        // Ours appended, matcher where wanted.
        assert_eq!(
            root["hooks"]["SessionStart"][1]["hooks"][0]["command"],
            "/usr/bin/converge hook inject"
        );
        assert_eq!(root["hooks"]["PreToolUse"][0]["matcher"], "mcp__converge__");

        // Idempotent.
        assert!(
            merge(&file, &wanted("/usr/bin/converge", ""))
                .unwrap()
                .is_empty()
        );

        // A moved binary updates the command in place, no duplicates.
        let changed = merge(&file, &wanted("/opt/converge", "")).unwrap();
        assert_eq!(changed.len(), 4);
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(root["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
        assert_eq!(
            root["hooks"]["SessionStart"][1]["hooks"][0]["command"],
            "/opt/converge hook inject"
        );

        // A flagged harness is a *different* entry, not a path update of
        // the unflagged one: the tails differ, so both can coexist.
        let changed = merge(&file, &wanted("/opt/converge", " --harness codex")).unwrap();
        assert_eq!(changed.len(), 4, "{changed:?}");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(root["hooks"]["SessionStart"].as_array().unwrap().len(), 3);
        assert!(
            merge(&file, &wanted("/opt/converge", " --harness codex"))
                .unwrap()
                .is_empty()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
