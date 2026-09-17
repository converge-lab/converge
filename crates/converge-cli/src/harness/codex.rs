//! Codex CLI: four hook commands in `~/.codex/hooks.json`, and an MCP
//! server entry in `~/.codex/config.toml`.
//!
//! Codex adopted Claude Code's hook contract wholesale — the same payload
//! fields arrive (`cwd`, `tool_input`, `tool_response`, `transcript_path`),
//! the same `hookSpecificOutput` envelope goes back (its own JSON Schema
//! spells it `SessionStartHookSpecificOutputWire`), and MCP tools carry
//! the same `mcp__server__tool` names the matchers key on. So only three
//! things here are actually Codex's own:
//!
//! 1. **Where installation lands** — a hooks file of its own, and an MCP
//!    entry we write into `config.toml`. `codex mcp add` can only take
//!    the bearer as `--bearer-token-env-var`, and an env var that has to
//!    be exported in every shell is not an integration; the `bearer_token`
//!    key next to it is rejected outright for streamable HTTP. What does
//!    work — and mirrors the header Claude Code stores — is
//!    `http_headers`, which Codex also redacts when printing the config.
//! 2. **The trust gate** — Codex refuses to run a non-managed hook until
//!    the human has reviewed and trusted that exact definition. Writing
//!    the file is not enough, so [`Harness::notes`] says so out loud.
//! 3. **The transcript** — rollout JSONL, parsed in [`crate::transcript`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use toml_edit::{DocumentMut, InlineTable, Item, Table, value};

use super::{Harness, Installed, Payload, Response, Transcript, cwd_or_current, hooks_file, wire};
use crate::config::Config;
use crate::transcript::{self, Parsed};

/// The MCP server name, in Codex's config and in ours.
const SERVER: &str = "converge";

pub struct CodexCli;

impl Harness for CodexCli {
    fn label(&self) -> &'static str {
        "Codex CLI"
    }

    fn detect(&self) -> bool {
        let in_path = Command::new("sh")
            .args(["-c", "command -v codex"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        in_path || home().is_some_and(|home| home.exists())
    }

    fn install(&self, exe: &str) -> Result<Installed> {
        let path = hooks_path().context("locate ~/.codex/hooks.json")?;
        let changed = hooks_file::merge(&path, &hooks_file::wanted(exe, " --harness codex"))?;
        Ok(Installed {
            noun: "hooks",
            changed,
            path,
        })
    }

    fn missing(&self, exe: &str) -> Vec<&'static str> {
        hooks_path()
            .map(|path| hooks_file::missing(&path, &hooks_file::wanted(exe, " --harness codex")))
            .unwrap_or_default()
    }

    fn notes(&self, _config: &Config) -> Vec<String> {
        vec![
            "one more step for Codex: it skips hooks nobody has vouched \
             for. Open Codex, run `/hooks`, and trust the five `converge \
             hook …` entries — until then they are installed but inert. \
             The trust is recorded against the exact hook definition, so \
             repeat it if `converge init` ever rewrites the commands (a \
             moved binary, say)."
                .to_string(),
        ]
    }

    fn mcp_registered(&self) -> bool {
        config_doc()
            .ok()
            .is_some_and(|doc| doc.get("mcp_servers").and_then(|m| m.get(SERVER)).is_some())
    }

    fn mcp_register(&self, config: &Config) -> Result<()> {
        let path = config_path().context("locate ~/.codex/config.toml")?;
        let mut doc = config_doc()?;

        // `mcp_servers` may be absent, or present as the dotted-key form;
        // either way we want an implicit table to hang `converge` off, so
        // an untouched neighbour server keeps its own formatting.
        let servers = doc
            .entry("mcp_servers")
            .or_insert_with(|| Item::Table(implicit()));
        let Some(servers) = servers.as_table_mut() else {
            anyhow::bail!("{}: `mcp_servers` is not a table", path.display());
        };
        servers.set_implicit(true);

        let mut headers = InlineTable::new();
        headers.insert("Authorization", format!("Bearer {}", config.token).into());
        let mut entry = Table::new();
        entry["url"] = value(format!("{}/mcp", config.server));
        entry["http_headers"] = value(headers);
        servers.insert(SERVER, Item::Table(entry));

        write_config(&path, &doc)
    }

    fn mcp_unregister(&self) -> Result<()> {
        let path = config_path().context("locate ~/.codex/config.toml")?;
        let mut doc = config_doc()?;
        if let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) {
            servers.remove(SERVER);
        }
        write_config(&path, &doc)
    }

    fn mcp_manual_hint(&self, config: &Config) -> String {
        format!(
            "codex mcp add {SERVER} --url {}/mcp   # then add `http_headers = \
             {{ Authorization = \"Bearer <token>\" }}` under \
             [mcp_servers.{SERVER}] in ~/.codex/config.toml",
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
        transcript::codex(path)
    }
}

fn home() -> Option<PathBuf> {
    // CODEX_HOME is how Codex itself relocates; honoring it keeps us
    // installing into the same place a relocated Codex reads from.
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".codex"))
}

fn hooks_path() -> Option<PathBuf> {
    home().map(|home| home.join("hooks.json"))
}

fn config_path() -> Option<PathBuf> {
    home().map(|home| home.join("config.toml"))
}

/// Codex's own config, parsed so edits preserve the comments and layout
/// around them — people hand-tune this file heavily.
fn config_doc() -> Result<DocumentMut> {
    let Some(path) = config_path() else {
        anyhow::bail!("neither CODEX_HOME nor HOME is set");
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    text.parse()
        .with_context(|| format!("{} is not valid TOML", path.display()))
}

fn write_config(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, doc.to_string()).with_context(|| format!("write {}", path.display()))
}

/// A table that renders only through its children — `[mcp_servers]` is
/// noise when the thing being configured is `[mcp_servers.converge]`.
fn implicit() -> Table {
    let mut table = Table::new();
    table.set_implicit(true);
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A scratch CODEX_HOME. Tests that set it run serially (one process,
    /// and the var is global) — they are all in this one function.
    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-codex-{}-{}",
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
    fn mcp_entry_round_trips_without_disturbing_the_rest() {
        let dir = scratch();
        // SAFETY: single-threaded within this test, and no other test in
        // this module reads CODEX_HOME.
        unsafe { std::env::set_var("CODEX_HOME", &dir) };

        // A config with the kind of hand-written content people keep here.
        std::fs::write(
            dir.join("config.toml"),
            "# my providers\nmodel = \"gpt-5\"\n\n[model_providers.local]\n\
             base_url = \"http://127.0.0.1:8094/v1\"\n",
        )
        .unwrap();

        let config = Config {
            server: "https://converge.example".into(),
            token: "cvg_secret".into(),
            auto_update: true,
        };

        assert!(!CodexCli.mcp_registered());
        CodexCli.mcp_register(&config).unwrap();
        assert!(CodexCli.mcp_registered());

        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        // Ours landed...
        assert!(text.contains("[mcp_servers.converge]"), "{text}");
        assert!(
            text.contains("url = \"https://converge.example/mcp\""),
            "{text}"
        );
        assert!(
            text.contains("Authorization = \"Bearer cvg_secret\""),
            "{text}"
        );
        // ...and theirs survived, comment and all.
        assert!(text.contains("# my providers"), "{text}");
        assert!(text.contains("[model_providers.local]"), "{text}");

        // Re-registering replaces rather than duplicates.
        CodexCli.mcp_register(&config).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert_eq!(text.matches("[mcp_servers.converge]").count(), 1, "{text}");

        CodexCli.mcp_unregister().unwrap();
        assert!(!CodexCli.mcp_registered());
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(text.contains("# my providers"), "{text}");

        // Hooks land in codex's own file, flagged so the entrypoint knows
        // whose payload it is reading.
        let installed = CodexCli.install("/usr/bin/converge").unwrap();
        assert_eq!(installed.changed.len(), 5);
        let hooks: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "/usr/bin/converge hook inject --harness codex"
        );
        assert_eq!(
            hooks["hooks"]["PreToolUse"][0]["matcher"],
            "mcp__converge__"
        );

        unsafe { std::env::remove_var("CODEX_HOME") };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_same_payload_fields_as_claude() {
        let parsed = CodexCli.parse(&json!({
            "cwd": "/repo",
            "tool_input": { "a": 1 },
            "tool_response": { "project_id": "x" },
            "transcript_path": "/rollout.jsonl",
        }));
        assert_eq!(parsed.cwd, PathBuf::from("/repo"));
        assert_eq!(parsed.tool_input["a"], 1);
        assert_eq!(
            parsed.transcript.as_ref().map(Transcript::key),
            Some("/rollout.jsonl".to_string())
        );
    }
}
