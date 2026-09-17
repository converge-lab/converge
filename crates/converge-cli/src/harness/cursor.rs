//! Cursor: four hook commands in `~/.cursor/hooks.json`, and an MCP
//! server in `~/.cursor/mcp.json`.
//!
//! The same four events as everyone else, in Cursor's own dialect:
//! camelCase event names (`sessionStart`), snake_case output
//! (`additional_context`, `updated_input`), a flat array of entries per
//! event rather than Claude's matcher groups, and `tool_output` where
//! the others say `tool_response`. Close enough to feel familiar, far
//! enough that nothing is shared but the entrypoints.
//!
//! **Untested against a real Cursor.** It is a GUI application, so it
//! cannot run in the container the other harnesses are exercised in; the
//! shapes below come from Cursor's published hook documentation and the
//! unit tests only pin what we write, not what Cursor accepts. Two
//! specifics to confirm on a real install are called out inline.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{Harness, Installed, Payload, Response, Transcript, cwd_or_current, json_file};
use crate::config::Config;
use crate::transcript::{self, Parsed};

const SERVER: &str = "converge";

pub struct Cursor;

impl Harness for Cursor {
    fn label(&self) -> &'static str {
        "Cursor"
    }

    fn detect(&self) -> bool {
        let in_path = Command::new("sh")
            .args(["-c", "command -v cursor"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        in_path || home().is_some_and(|home| home.exists())
    }

    fn install(&self, exe: &str) -> Result<Installed> {
        let path = hooks_path().context("locate ~/.cursor/hooks.json")?;
        let changed = merge_hooks(&path, exe)?;
        Ok(Installed {
            noun: "hooks",
            changed,
            path,
        })
    }

    fn notes(&self, _config: &Config) -> Vec<String> {
        vec![
            "note for Cursor: its hooks run for every tool call, because \
             the tool-type string its `matcher` expects for MCP tools is \
             not something we can verify from here. `converge hook ctx` \
             checks the tool name itself and stays out of the way of \
             anything that is not converge's, so this is a few \
             milliseconds per call, not a correctness problem."
                .to_string(),
        ]
    }

    fn mcp_registered(&self) -> bool {
        mcp_json()
            .ok()
            .is_some_and(|(doc, _)| doc["mcpServers"].get(SERVER).is_some())
    }

    fn mcp_register(&self, config: &Config) -> Result<()> {
        let path = mcp_path().context("locate ~/.cursor/mcp.json")?;
        let (mut doc, like) = mcp_json()?;
        if !doc.is_object() {
            bail!("{} is not a JSON object", path.display());
        }
        if !doc["mcpServers"].is_object() {
            doc["mcpServers"] = json!({});
        }
        doc["mcpServers"][SERVER] = json!({
            "url": format!("{}/mcp", config.server),
            "headers": { "Authorization": format!("Bearer {}", config.token) },
        });
        json_file::write(&path, &doc, &like)
    }

    fn mcp_unregister(&self) -> Result<()> {
        let path = mcp_path().context("locate ~/.cursor/mcp.json")?;
        let (mut doc, like) = mcp_json()?;
        if let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) {
            servers.remove(SERVER);
        }
        json_file::write(&path, &doc, &like)
    }

    fn mcp_manual_hint(&self, config: &Config) -> String {
        format!(
            "add to `mcpServers` in ~/.cursor/mcp.json:  \"{SERVER}\": {{ \
             \"url\": \"{}/mcp\", \"headers\": {{ \"Authorization\": \
             \"Bearer <token>\" }} }}",
            config.server
        )
    }

    fn parse(&self, raw: &Value) -> Payload {
        // `cwd` only rides the tool events; session events carry
        // `workspace_roots` instead, and the first root is the session's
        // repository.
        let cwd = raw["cwd"]
            .as_str()
            .or_else(|| raw["workspace_roots"][0].as_str());
        Payload {
            cwd: cwd_or_current(cwd),
            tool_name: raw["tool_name"].as_str().map(str::to_owned),
            tool_input: raw["tool_input"].clone(),
            // Cursor says `tool_output` where the others say
            // `tool_response`.
            tool_response: raw["tool_output"].clone(),
            // Null unless the user has transcripts enabled, in which
            // case evidence sync simply has nothing to read.
            transcript: raw["transcript_path"]
                .as_str()
                .map(|path| Transcript::File(PathBuf::from(path))),
        }
    }

    fn emit(&self, response: Response) -> Option<Value> {
        Some(match response {
            // No `systemMessage` equivalent on sessionStart, so the
            // visible line is dropped rather than smuggled into the
            // model's context.
            Response::Inject { context, .. } => json!({ "additional_context": context }),
            Response::Ctx { tool_input } => json!({ "updated_input": tool_input }),
            // CONFIRM ON A REAL CURSOR: `user_message` is documented for
            // preToolUse; on postToolUse and sessionEnd it is most
            // likely ignored, which is the harmless outcome.
            Response::Notice { system } => json!({ "user_message": system }),
            Response::Marked {
                system: Some(system),
                ..
            } => json!({ "user_message": system }),
            Response::Marked { system: None, .. } | Response::Silent => return None,
        })
    }

    fn transcript(&self, at: &Transcript) -> Result<Parsed> {
        let Transcript::File(path) = at else {
            bail!("Cursor records conversations to a file, not a session id");
        };
        // CONFIRM ON A REAL CURSOR: the transcript format is
        // undocumented and unavailable from here. Claude's JSONL is the
        // closest published shape; a mismatch surfaces as "sync skipped",
        // never as lost or corrupted evidence.
        transcript::claude(path)
    }
}

fn home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".cursor"))
}

fn hooks_path() -> Option<PathBuf> {
    home().map(|home| home.join("hooks.json"))
}

fn mcp_path() -> Option<PathBuf> {
    home().map(|home| home.join("mcp.json"))
}

/// The four events, in Cursor's spelling. No `matcher`: the tool-type
/// string it wants for MCP tools cannot be verified from here, and a
/// matcher that is subtly wrong means the integration silently never
/// runs. Firing for every tool and filtering inside `hook ctx` costs a
/// process spawn per call and cannot be silently wrong.
fn wanted(exe: &str) -> [(&'static str, String); 4] {
    let command = |sub: &str| format!("{exe} hook {sub} --harness cursor");
    [
        ("sessionStart", command("inject")),
        ("sessionEnd", command("sync")),
        ("preToolUse", command("ctx")),
        ("postToolUse", command("mark")),
    ]
}

/// Cursor keys each event to a flat array of entries, so this is its own
/// merge rather than [`super::hooks_file`]'s matcher groups. Same rules:
/// never touch what we did not write, recognise our own entry by the
/// `hook …` tail so a moved binary updates in place.
fn merge_hooks(path: &std::path::Path, exe: &str) -> Result<Vec<String>> {
    let (mut root, like) = json_file::read(path, json!({ "version": 1 }))?;
    if !root.is_object() {
        bail!("{} is not a JSON object", path.display());
    }
    if root["version"].is_null() {
        root["version"] = json!(1);
    }
    if !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }

    let mut changed = Vec::new();
    for (event, command) in wanted(exe) {
        let at = command.find(" hook ").expect("commands contain ` hook `");
        let tail = &command[at..];
        let entries = &mut root["hooks"][event];
        if !entries.is_array() {
            *entries = json!([]);
        }
        let list = entries.as_array_mut().expect("just ensured");

        let mut found = false;
        for entry in list.iter_mut() {
            if entry["command"].as_str().is_some_and(|c| c.ends_with(tail)) {
                found = true;
                if entry["command"].as_str() != Some(command.as_str()) {
                    entry["command"] = json!(command);
                    changed.push(format!("{event} (path updated)"));
                }
            }
        }
        if !found {
            list.push(json!({ "command": command }));
            changed.push(event.to_string());
        }
    }

    if !changed.is_empty() {
        json_file::write(path, &root, &like)?;
    }
    Ok(changed)
}

fn mcp_json() -> Result<(Value, String)> {
    let Some(path) = mcp_path() else {
        bail!("HOME is not set");
    };
    json_file::read(&path, json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_and_mcp_are_cursors_shapes() {
        let dir = std::env::temp_dir().join(format!(
            "cvg-cursor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let hooks = dir.join("hooks.json");

        // Someone else's hook, which must survive.
        std::fs::write(
            &hooks,
            r#"{"version":1,"hooks":{"sessionStart":[{"command":"./mine.sh"}]}}"#,
        )
        .unwrap();

        let changed = merge_hooks(&hooks, "/usr/bin/converge").unwrap();
        assert_eq!(changed.len(), 4, "{changed:?}");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();
        assert_eq!(root["version"], 1);
        assert_eq!(root["hooks"]["sessionStart"][0]["command"], "./mine.sh");
        assert_eq!(
            root["hooks"]["sessionStart"][1]["command"],
            "/usr/bin/converge hook inject --harness cursor"
        );
        // Flat entries, not matcher groups: no nested "hooks" key.
        assert!(root["hooks"]["preToolUse"][0]["hooks"].is_null());
        assert!(merge_hooks(&hooks, "/usr/bin/converge").unwrap().is_empty());
        assert_eq!(merge_hooks(&hooks, "/opt/converge").unwrap().len(), 4);
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();
        assert_eq!(root["hooks"]["sessionStart"].as_array().unwrap().len(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn speaks_snake_case_and_reads_tool_output() {
        let parsed = Cursor.parse(&json!({
            "workspace_roots": ["/repo"],
            "tool_name": "mcp__converge__project_bind",
            "tool_output": { "project_id": "x" },
            "transcript_path": "/t.jsonl",
        }));
        // No `cwd` on session events — the first workspace root stands in.
        assert_eq!(parsed.cwd, PathBuf::from("/repo"));
        assert_eq!(parsed.tool_response["project_id"], "x");
        assert_eq!(
            parsed.tool_name.as_deref(),
            Some("mcp__converge__project_bind")
        );

        let inject = Cursor
            .emit(Response::Inject {
                context: "ctx".into(),
                system: "dropped".into(),
                sticky: true,
                degraded: false,
                state: "bound",
            })
            .unwrap();
        assert_eq!(inject["additional_context"], "ctx");
        assert!(inject.get("hookSpecificOutput").is_none());
        assert!(inject.get("systemMessage").is_none());

        let ctx = Cursor
            .emit(Response::Ctx {
                tool_input: json!({ "cwd": "/repo" }),
            })
            .unwrap();
        assert_eq!(ctx["updated_input"]["cwd"], "/repo");
    }
}
