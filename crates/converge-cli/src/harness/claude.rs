//! Claude Code: four hook commands in `~/.claude/settings.json`, and an
//! MCP registration made through the `claude` CLI.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{Harness, Installed, Payload, Response, Transcript, cwd_or_current, hooks_file, wire};
use crate::config::Config;
use crate::transcript::{self, Parsed};

pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn detect(&self) -> bool {
        let in_path = Command::new("sh")
            .args(["-c", "command -v claude"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        in_path || settings_path().map(|p| p.parent().is_some_and(|d| d.exists())) == Some(true)
    }

    fn install(&self, exe: &str) -> Result<Installed> {
        let path = settings_path().context("locate ~/.claude/settings.json")?;
        // No `--harness` suffix: every command already installed out
        // there omits the flag, and the default has to keep matching it.
        let changed = hooks_file::merge(&path, &hooks_file::wanted(exe, ""))?;
        Ok(Installed {
            noun: "hooks",
            changed,
            path,
        })
    }

    fn missing(&self, exe: &str) -> Vec<&'static str> {
        settings_path()
            .map(|path| hooks_file::missing(&path, &hooks_file::wanted(exe, "")))
            .unwrap_or_default()
    }

    fn ask_tool(&self) -> Option<&'static str> {
        Some("AskUserQuestion")
    }

    fn notes(&self, config: &Config) -> Vec<String> {
        // Account connectors can't be added programmatically (claude.ai
        // UI only; they sync down to Claude Code, never up) — the best we
        // can do is point at the documented settings page.
        vec![format!(
            "want converge on claude.ai web and mobile too? Add a custom \
             connector at https://claude.ai/customize/connectors with URL \
             {}/mcp — it signs in via your browser and also appears in \
             Claude Code automatically.",
            config.server
        )]
    }

    fn mcp_registered(&self) -> bool {
        Command::new("claude")
            .args(["mcp", "get", "converge"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn mcp_register(&self, config: &Config) -> Result<()> {
        quiet_claude(&[
            "mcp",
            "add",
            "--transport",
            "http",
            "--scope",
            "user",
            "converge",
            &format!("{}/mcp", config.server),
            "--header",
            &format!("Authorization: Bearer {}", config.token),
        ])
    }

    fn mcp_unregister(&self) -> Result<()> {
        quiet_claude(&["mcp", "remove", "--scope", "user", "converge"])
    }

    fn mcp_manual_hint(&self, config: &Config) -> String {
        format!(
            "claude mcp add --transport http --scope user converge {}/mcp \
             --header \"Authorization: Bearer <token>\"",
            config.server
        )
    }

    fn parse(&self, raw: &Value) -> Payload {
        Payload {
            cwd: cwd_or_current(raw["cwd"].as_str()),
            tool_name: raw["tool_name"].as_str().map(str::to_owned),
            tool_input: raw["tool_input"].clone(),
            tool_response: raw["tool_response"].clone(),
            transcript: raw["transcript_path"]
                .as_str()
                .map(|path| Transcript::File(PathBuf::from(path))),
            event: raw["hook_event_name"].as_str().map(str::to_owned),
            session: raw["session_id"].as_str().map(str::to_owned),
        }
    }

    fn emit(&self, response: Response) -> Option<Value> {
        wire::emit(response)
    }

    fn transcript(&self, at: &Transcript) -> Result<Parsed> {
        let Transcript::File(path) = at else {
            bail!(
                "{} records conversations to a file, not a session id",
                self.label()
            );
        };
        transcript::claude(path)
    }
}

fn settings_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".claude/settings.json"))
}

/// Run the claude CLI with its chatter captured: our own status line is
/// the UX; claude's output surfaces only when the command fails.
fn quiet_claude(args: &[&str]) -> Result<()> {
    let output = Command::new("claude")
        .args(args)
        .output()
        .with_context(|| format!("run `claude {} …` (is the claude CLI installed?)", args[0]))?;
    if !output.status.success() {
        bail!(
            "`claude {} {}` failed with {}:\n{}{}",
            args[0],
            args[1],
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payload_and_output_shapes_are_claudes() {
        let parsed = ClaudeCode.parse(&json!({
            "cwd": "/repo",
            "tool_input": { "a": 1 },
            "tool_response": { "project_id": "x" },
            "transcript_path": "/t.jsonl",
        }));
        assert_eq!(parsed.cwd, PathBuf::from("/repo"));
        assert_eq!(parsed.tool_input["a"], 1);
        assert_eq!(parsed.tool_response["project_id"], "x");
        assert_eq!(
            parsed.transcript.as_ref().map(Transcript::key),
            Some("/t.jsonl".to_string())
        );

        // Claude nests both the context block and the rewritten input
        // under hookSpecificOutput; nothing else there does.
        let inject = ClaudeCode
            .emit(Response::Inject {
                context: "ctx".into(),
                system: "line".into(),
                sticky: true,
                degraded: false,
                state: "bound",
            })
            .unwrap();
        assert_eq!(inject["systemMessage"], "line");
        assert_eq!(inject["hookSpecificOutput"]["additionalContext"], "ctx");
        assert_eq!(
            inject["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );

        let ctx = ClaudeCode
            .emit(Response::Ctx {
                tool_input: json!({ "cwd": "/repo" }),
            })
            .unwrap();
        assert_eq!(ctx["hookSpecificOutput"]["updatedInput"]["cwd"], "/repo");

        assert!(ClaudeCode.emit(Response::Silent).is_none());
    }
}
