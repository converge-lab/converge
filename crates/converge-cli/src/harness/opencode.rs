//! opencode: a generated TypeScript-ish plugin, and an MCP entry in
//! `opencode.json`.
//!
//! The odd one out. opencode has no hook commands at all — plugins are
//! JavaScript modules loaded into its own process — so installation
//! writes a shim (`opencode.js`, next to this file) that calls the same
//! `converge hook …` entrypoints every other harness invokes directly.
//! Three consequences shape the code below:
//!
//! - **No session-start event.** The shim injects through
//!   `experimental.chat.system.transform`, which runs while the system
//!   prompt is assembled — for *every* request, since that prompt is
//!   rebuilt each time rather than kept in the conversation. So the block
//!   is pushed on every request and `hook inject` is cached per session.
//! - **No session-end event.** Evidence syncs on `session.idle`, i.e.
//!   every lull. Watermarks make that cheap and it loses less than a
//!   single end-of-session push would.
//! - **No transcript file.** Sessions live in SQLite; `opencode export`
//!   is the way out, so this is the first [`Transcript::Session`].

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{Harness, Installed, Payload, Response, Transcript, cwd_or_current, json_file};
use crate::config::Config;
use crate::transcript::{self, Parsed};

const SERVER: &str = "converge";

/// The shim, with the binary path still to be filled in.
const SHIM: &str = include_str!("opencode.js");
const BIN_PLACEHOLDER: &str = "__CONVERGE_BIN__";

pub struct OpenCode;

impl Harness for OpenCode {
    fn label(&self) -> &'static str {
        "opencode"
    }

    fn detect(&self) -> bool {
        let in_path = Command::new("sh")
            .args(["-c", "command -v opencode"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        in_path || config_dir().is_some_and(|dir| dir.exists())
    }

    fn install(&self, exe: &str) -> Result<Installed> {
        let path = plugin_path().context("locate the opencode plugin directory")?;
        let shim = SHIM.replace(BIN_PLACEHOLDER, exe);
        // Rewrite only on change, so `converge init` reports honestly
        // rather than claiming work it did not do.
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        let changed = if current == shim {
            Vec::new()
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("create {}", dir.display()))?;
            }
            std::fs::write(&path, &shim).with_context(|| format!("write {}", path.display()))?;
            vec!["converge.js".to_string()]
        };
        Ok(Installed {
            noun: "plugin",
            changed,
            path,
        })
    }

    fn artifacts(&self) -> Vec<PathBuf> {
        plugin_path().into_iter().chain(config_path()).collect()
    }

    fn integrated(&self, _exe: &str) -> bool {
        plugin_path().is_some_and(|path| path.exists())
    }

    fn after_refresh(&self) -> Option<&'static str> {
        Some("restart opencode to load it")
    }

    fn ask_tool(&self) -> Option<&'static str> {
        // Registered as `question`; not every agent is allowed it, which
        // is why the wording treats it as a preference.
        Some("question")
    }

    fn notes(&self, _config: &Config) -> Vec<String> {
        vec![
            "note for opencode: it has no session-start hook, so the \
             context block rides `experimental.chat.system.transform`. \
             If a future opencode drops that API, sessions stop getting \
             the decision index — the tools keep working, and `converge \
             update` will carry the fix. Its status lines (\"linked this \
             repo\", sync counts, write failures) show as TUI toasts; a \
             headless `opencode run`/`serve` has nowhere to show them."
                .to_string(),
        ]
    }

    fn mcp_registered(&self) -> bool {
        config_json()
            .ok()
            .is_some_and(|(doc, _)| doc["mcp"].get(SERVER).is_some())
    }

    fn mcp_register(&self, config: &Config) -> Result<()> {
        let path = config_path().context("locate opencode.json")?;
        let (mut doc, like) = config_json()?;
        if !doc.is_object() {
            bail!("{} is not a JSON object", path.display());
        }
        if !doc["mcp"].is_object() {
            doc["mcp"] = json!({});
        }
        doc["mcp"][SERVER] = json!({
            "type": "remote",
            "url": format!("{}/mcp", config.server),
            "enabled": true,
            "headers": { "Authorization": format!("Bearer {}", config.token) },
        });
        json_file::write(&path, &doc, &like)
    }

    fn mcp_unregister(&self) -> Result<()> {
        let path = config_path().context("locate opencode.json")?;
        let (mut doc, like) = config_json()?;
        if let Some(mcp) = doc.get_mut("mcp").and_then(Value::as_object_mut) {
            mcp.remove(SERVER);
        }
        json_file::write(&path, &doc, &like)
    }

    fn mcp_manual_hint(&self, config: &Config) -> String {
        format!(
            "add to the `mcp` object in opencode.json:  \"{SERVER}\": {{ \
             \"type\": \"remote\", \"url\": \"{}/mcp\", \"enabled\": true, \
             \"headers\": {{ \"Authorization\": \"Bearer <token>\" }} }}",
            config.server
        )
    }

    fn parse(&self, raw: &Value) -> Payload {
        Payload {
            cwd: cwd_or_current(raw["cwd"].as_str()),
            // The shim has already filtered to converge's tools.
            tool_name: None,
            tool_input: raw["tool_input"].clone(),
            tool_response: raw["tool_response"].clone(),
            event: None,
            session: raw["session_id"].as_str().map(str::to_owned),
            // Not a file: opencode keeps the conversation in SQLite and
            // identifies it by session id.
            transcript: raw["session_id"]
                .as_str()
                .map(|id| Transcript::Session(id.to_string())),
        }
    }

    fn emit(&self, response: Response) -> Option<Value> {
        // Our own shape, because both ends are ours: the shim reads
        // exactly these keys. No `hookSpecificOutput` envelope to ape —
        // that belongs to harnesses that defined one.
        Some(match response {
            Response::Inject {
                context,
                system,
                sticky,
                degraded,
                state,
            } => json!({
                "context": context,
                "system": system,
                "sticky": sticky,
                "degraded": degraded,
                "state": state,
            }),
            Response::Signals {
                context, system, ..
            } => json!({ "context": context, "system": system }),
            Response::Ctx { tool_input } => json!({ "tool_input": tool_input }),
            Response::Notice { system } => json!({ "system": system }),
            Response::Marked { effect, system } => json!({
                "effect": effect.as_str(),
                "system": system,
            }),
            Response::Silent => return None,
        })
    }

    fn transcript(&self, at: &Transcript) -> Result<Parsed> {
        let Transcript::Session(id) = at else {
            bail!("opencode identifies conversations by session id, not by path");
        };
        // `opencode export` prints the session as JSON on stdout; its
        // progress chatter goes to stderr.
        let out = Command::new("opencode")
            .args(["export", id])
            .output()
            .context("run `opencode export …` (is opencode installed?)")?;
        if !out.status.success() {
            bail!(
                "`opencode export {id}` failed with {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        transcript::opencode(&out.stdout)
    }
}

/// Where opencode keeps global configuration. `OPENCODE_CONFIG_DIR` is
/// how opencode itself relocates it, and it governs plugin discovery
/// too, so honoring it keeps us installing where opencode reads.
fn config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OPENCODE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.config")))
        .ok()?;
    Some(PathBuf::from(base).join("opencode"))
}

/// opencode loads `plugin/` and `plugins/` alike; one is enough.
fn plugin_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("plugin").join("converge.js"))
}

fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("opencode.json"))
}

/// The config plus its raw text, so a write can match the owner's
/// formatting — this file is hand-maintained and hundreds of lines.
fn config_json() -> Result<(Value, String)> {
    let Some(path) = config_path() else {
        bail!("neither OPENCODE_CONFIG_DIR, XDG_CONFIG_HOME nor HOME is set");
    };
    json_file::read(
        &path,
        json!({ "$schema": "https://opencode.ai/config.json" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_mcp_entry_leave_the_rest_of_the_config_alone() {
        let dir = std::env::temp_dir().join(format!(
            "cvg-oc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the only test in this module touching the variable.
        unsafe { std::env::set_var("OPENCODE_CONFIG_DIR", &dir) };

        std::fs::write(
            dir.join("opencode.json"),
            r#"{"$schema":"https://opencode.ai/config.json","model":"zzz/m","provider":{"local":{"name":"mine"}}}"#,
        )
        .unwrap();

        let config = Config {
            server: "https://converge.example".into(),
            token: "cvg_secret".into(),
            auto_update: true,
        };

        // The plugin lands with the real path baked in, and says so once.
        let installed = OpenCode.install("/usr/bin/converge").unwrap();
        assert_eq!(installed.noun, "plugin");
        assert_eq!(installed.changed, vec!["converge.js".to_string()]);
        let shim = std::fs::read_to_string(dir.join("plugin/converge.js")).unwrap();
        assert!(
            shim.contains(r#"const CONVERGE = "/usr/bin/converge""#),
            "{shim}"
        );
        assert!(!shim.contains(BIN_PLACEHOLDER));
        // Re-running is a no-op that reports nothing.
        assert!(
            OpenCode
                .install("/usr/bin/converge")
                .unwrap()
                .changed
                .is_empty()
        );
        // A moved binary rewrites it.
        assert_eq!(OpenCode.install("/opt/converge").unwrap().changed.len(), 1);

        assert!(!OpenCode.mcp_registered());
        OpenCode.mcp_register(&config).unwrap();
        assert!(OpenCode.mcp_registered());

        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(doc["mcp"]["converge"]["type"], "remote");
        assert_eq!(
            doc["mcp"]["converge"]["url"],
            "https://converge.example/mcp"
        );
        assert_eq!(
            doc["mcp"]["converge"]["headers"]["Authorization"],
            "Bearer cvg_secret"
        );
        // Their keys survive, and in their order — preserve_order earns
        // its place here.
        assert_eq!(doc["model"], "zzz/m");
        assert_eq!(doc["provider"]["local"]["name"], "mine");
        let keys: Vec<_> = doc.as_object().unwrap().keys().cloned().collect();
        assert_eq!(&keys[..3], &["$schema", "model", "provider"], "{keys:?}");

        OpenCode.mcp_unregister().unwrap();
        assert!(!OpenCode.mcp_registered());

        unsafe { std::env::remove_var("OPENCODE_CONFIG_DIR") };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_session_id_is_not_a_path() {
        let parsed = OpenCode.parse(&json!({
            "cwd": "/repo",
            "session_id": "ses_abc",
            "tool_input": { "a": 1 },
        }));
        assert!(matches!(
            parsed.transcript,
            Some(Transcript::Session(ref id)) if id == "ses_abc"
        ));
        // And the shim's flat answer, not Claude's envelope.
        let ctx = OpenCode
            .emit(Response::Ctx {
                tool_input: json!({ "cwd": "/repo" }),
            })
            .unwrap();
        assert_eq!(ctx["tool_input"]["cwd"], "/repo");
        assert!(ctx.get("hookSpecificOutput").is_none());
    }
}
