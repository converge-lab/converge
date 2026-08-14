//! `converge init` — the one interactive moment, once per machine:
//! credentials → agent-tool integration (hooks + MCP registration).
//! Idempotent: every step detects "already done" and moves on, so
//! re-running after an upgrade or a moved binary is the repair path.
//!
//! After this, the per-repository flow is entirely in-session: hooks
//! surface suggestions, the human answers in conversation, hooks write
//! the marker. No further terminal rituals.

use std::io::{BufRead, IsTerminal, Write as _};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::device;

/// The managed cloud — the Enter-accepts default of the server prompt.
/// Self-hosters type their own URL over it.
const HOSTED: &str = "https://app.converge.expert";

/// On a terminal, prompts are styled (dialoguer); on piped stdin they
/// stay plain line reads — scripts and the e2e harness depend on that.
fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// One line of input; `default` fills an empty answer (and renders as
/// the editable/bracketed default on a terminal).
fn input(prompt: &str, default: Option<&str>) -> Result<String> {
    if interactive() {
        let theme = dialoguer::theme::ColorfulTheme::default();
        let mut q = dialoguer::Input::<String>::with_theme(&theme).with_prompt(prompt);
        if let Some(default) = default {
            q = q.default(default.to_string());
        }
        Ok(q.interact_text()?)
    } else {
        let answer = plain(prompt)?;
        Ok(match (answer.is_empty(), default) {
            (true, Some(default)) => default.to_string(),
            _ => answer,
        })
    }
}

/// A secret: hidden while typing on a terminal, plain on a pipe.
fn secret(prompt: &str) -> Result<String> {
    if interactive() {
        let theme = dialoguer::theme::ColorfulTheme::default();
        Ok(dialoguer::Password::with_theme(&theme)
            .with_prompt(prompt)
            .interact()?)
    } else {
        plain(prompt)
    }
}

/// A yes/no question; empty answer takes the default.
fn confirm(prompt: &str, default: bool) -> Result<bool> {
    if interactive() {
        let theme = dialoguer::theme::ColorfulTheme::default();
        Ok(dialoguer::Confirm::with_theme(&theme)
            .with_prompt(prompt)
            .default(default)
            .interact()?)
    } else {
        let answer = plain(&format!(
            "{prompt} [{}]",
            if default { "Y/n" } else { "y/N" }
        ))?;
        Ok(match answer.as_str() {
            "" => default,
            a => a.eq_ignore_ascii_case("y"),
        })
    }
}

/// The pipe-mode primitive: print the prompt, read one line. EOF is a
/// hard stop — retry loops around empty answers would otherwise spin
/// forever on a closed stdin.
fn plain(prompt: &str) -> Result<String> {
    print!("{prompt}: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("read stdin")?;
    if read == 0 {
        bail!("stdin closed before `{prompt}` was answered");
    }
    Ok(line.trim().to_string())
}

pub async fn run(force: bool) -> Result<()> {
    // ── credentials ────────────────────────────────────────────────────
    let config = match Config::load() {
        Ok(config) if force => {
            // Reinit: redo credentials even though the stored ones may
            // work — the path for switching servers or identities. The
            // replaced credential stays valid server-side; revoke it in
            // Settings if it shouldn't outlive this machine's config.
            println!("reconfiguring — was {}", config.server);
            credentials().await?
        }
        Ok(config) => match verified(&config).await {
            Ok(handle) => {
                println!("✓ credentials: {} as @{handle}", config.server);
                config
            }
            Err(e) => {
                println!("configured, but not working ({e}) — let's redo it");
                credentials().await?
            }
        },
        Err(_) => credentials().await?,
    };

    if let Some(warning) = crate::skew::check(&config.client()?).await {
        println!("⚠ {warning}");
    }

    // ── agent tool ─────────────────────────────────────────────────────
    if !claude_code_present() {
        println!(
            "\nno Claude Code found (no `claude` in PATH, no ~/.claude). \
             Install it, then re-run `converge init`; other agent tools: \
             wire the four hook commands (`converge hook inject|ctx|mark|sync`) \
             and add the MCP server {}/mcp manually.",
            config.server
        );
        return Ok(());
    }
    let exe = std::env::current_exe().context("resolve own path")?;
    let settings = claude_settings_path().context("locate ~/.claude/settings.json")?;
    let changed = install_hooks(&settings, &exe.to_string_lossy())?;
    if changed.is_empty() {
        println!("✓ hooks: already installed ({})", settings.display());
    } else {
        println!(
            "✓ hooks: installed {} ({})",
            changed.join(", "),
            settings.display()
        );
    }

    // ── MCP registration ───────────────────────────────────────────────
    // A registration bakes in the server URL and bearer, so a forced
    // reinit must replace it, not keep it.
    let registered = mcp_registered();
    if registered && !force {
        println!("✓ mcp: `converge` server already registered");
    } else if confirm("register the MCP server with Claude Code?", true)? {
        if registered {
            unregister_mcp()?;
        }
        register_mcp(&config)?;
        println!("✓ mcp: registered {}/mcp as `converge`", config.server);
    } else {
        println!(
            "skipped — register later with:\n  claude mcp add --transport http \
             --scope user converge {}/mcp --header \"Authorization: Bearer <token>\"",
            config.server
        );
    }

    println!(
        "\ndone. Open any repository in Claude Code — the session will \
         suggest a project binding (or run `converge project init` yourself)."
    );
    // Account connectors can't be added programmatically (claude.ai UI
    // only; they sync down to Claude Code, never up) — the best we can
    // do is point at the documented settings page.
    println!(
        "\nwant converge on claude.ai web and mobile too? Add a custom \
         connector at https://claude.ai/customize/connectors with URL \
         {}/mcp — it signs in via your browser and also appears in \
         Claude Code automatically.",
        config.server
    );
    Ok(())
}

/// Obtain and store a credential: the browser-pairing device flow when
/// the server offers it (and stdin is a terminal — a human has to reach
/// a browser), the paste-a-token prompt otherwise.
async fn credentials() -> Result<Config> {
    println!("\nconverge setup — where is your server? (Enter = Converge Cloud)");
    let server = loop {
        let server = input("server URL", Some(HOSTED))?;
        if !server.is_empty() {
            break server.trim_end_matches('/').to_string();
        }
    };

    if interactive()
        && let Some(offer) = device::probe(&server).await
    {
        match device::pair(&offer).await {
            Ok(token) => return stored(Config { server, token }).await,
            Err(e) => println!("pairing failed ({e}) — falling back to a pasted token"),
        }
    }

    println!(
        "mint a token: open {server}/ → Settings → Create token \
         (or `converge-server token mint` on the server host)"
    );
    let token = loop {
        let token = secret("token (cvg_…)")?;
        if !token.is_empty() {
            break token;
        }
    };
    stored(Config { server, token }).await
}

/// Verify a credential end-to-end, then persist it (0600).
async fn stored(config: Config) -> Result<Config> {
    let handle = verified(&config)
        .await
        .with_context(|| format!("cannot reach {} with that token", config.server))?;
    let path = config::write(&config.server, &config.token)?;
    println!("✓ credentials: @{handle}; wrote {} (0600)", path.display());
    Ok(config)
}

/// One `/users/me` round trip proves server and token together.
async fn verified(config: &Config) -> Result<String> {
    Ok(config.client()?.me().await?.handle)
}

fn claude_code_present() -> bool {
    let in_path = Command::new("sh")
        .args(["-c", "command -v claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    in_path || claude_settings_path().map(|p| p.parent().is_some_and(|d| d.exists())) == Some(true)
}

fn claude_settings_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".claude/settings.json"))
}

/// The four hook registrations this integration needs. `exe` is this
/// binary's absolute path — re-run `converge init` after moving it.
fn wanted(exe: &str) -> [(&'static str, Option<&'static str>, String); 4] {
    [
        ("SessionStart", None, format!("{exe} hook inject")),
        ("SessionEnd", None, format!("{exe} hook sync")),
        (
            "PreToolUse",
            Some("mcp__converge__"),
            format!("{exe} hook ctx"),
        ),
        (
            "PostToolUse",
            Some("mcp__converge__(project_bind|project_dismiss)"),
            format!("{exe} hook mark"),
        ),
    ]
}

/// Merge our hooks into the settings file, conservatively: existing
/// content is never touched; an entry whose command ends with the same
/// `converge hook …` subcommand counts as present (so a moved binary
/// updates in place). Returns which events changed.
fn install_hooks(settings: &std::path::Path, exe: &str) -> Result<Vec<String>> {
    let mut root: Value = match std::fs::read_to_string(settings) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", settings.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e).with_context(|| format!("read {}", settings.display())),
    };
    if !root.is_object() {
        bail!("{} is not a JSON object", settings.display());
    }
    if !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }

    let mut changed = Vec::new();
    for (event, matcher, command) in wanted(exe) {
        let suffix = command
            .rsplit_once(" hook ")
            .map(|(_, sub)| format!(" hook {sub}"))
            .expect("wanted commands contain ` hook `");
        let entries = &mut root["hooks"][event];
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
                let is_ours = hook["command"]
                    .as_str()
                    .is_some_and(|c| c.ends_with(&suffix));
                if is_ours {
                    found = true;
                    if hook["command"].as_str() != Some(command.as_str()) {
                        hook["command"] = json!(command);
                        changed.push(format!("{event} (path updated)"));
                    }
                }
            }
        }
        if !found {
            let mut group = json!({ "hooks": [{ "type": "command", "command": command }] });
            if let Some(matcher) = matcher {
                group["matcher"] = json!(matcher);
            }
            list.push(group);
            changed.push(event.to_string());
        }
    }

    if !changed.is_empty() {
        if let Some(dir) = settings.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(settings, serde_json::to_string_pretty(&root)?)
            .with_context(|| format!("write {}", settings.display()))?;
    }
    Ok(changed)
}

/// Is a `converge` MCP server already known to Claude Code?
fn mcp_registered() -> bool {
    Command::new("claude")
        .args(["mcp", "get", "converge"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Drop the existing `converge` registration (reinit replaces it).
fn unregister_mcp() -> Result<()> {
    quiet_claude(&["mcp", "remove", "--scope", "user", "converge"])
}

fn register_mcp(config: &Config) -> Result<()> {
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

    #[test]
    fn hook_install_is_conservative_and_idempotent() {
        let dir = std::env::temp_dir().join(format!("cvg-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");

        // Existing user content that must survive untouched.
        std::fs::write(
            &settings,
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

        let changed = install_hooks(&settings, "/usr/bin/converge").unwrap();
        assert_eq!(changed.len(), 4, "{changed:?}");

        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
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
            install_hooks(&settings, "/usr/bin/converge")
                .unwrap()
                .is_empty()
        );

        // A moved binary updates the command in place, no duplicates.
        let changed = install_hooks(&settings, "/opt/converge").unwrap();
        assert_eq!(changed.len(), 4);
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(root["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
        assert_eq!(
            root["hooks"]["SessionStart"][1]["hooks"][0]["command"],
            "/opt/converge hook inject"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
