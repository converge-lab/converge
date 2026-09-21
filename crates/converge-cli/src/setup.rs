//! `converge init` — the one interactive moment, once per machine:
//! credentials → agent-tool integration (hooks or plugin, + MCP
//! registration), for every agent tool found on the machine.
//! Idempotent: every step detects "already done" and moves on, so
//! re-running after an upgrade or a moved binary is the repair path.
//!
//! After this, the per-repository flow is entirely in-session: hooks
//! surface suggestions, the human answers in conversation, hooks write
//! the marker. No further terminal rituals.

use std::io::{BufRead, IsTerminal, Write as _};

use std::path::PathBuf;

use crate::backup::Snapshot;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::device;
use crate::harness::{self, Harness, Kind};

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

pub async fn run(force: bool, only: Vec<Kind>) -> Result<()> {
    // ── credentials ───────────────────────────────────────────────────────
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

    // ── agent tools ──────────────────────────────────────────────────
    let Some(chosen) = chosen(&only, &config)? else {
        return Ok(());
    };

    let exe = std::env::current_exe().context("resolve own path")?;
    let exe = exe.to_string_lossy();
    // Every file about to be written is kept first, so this init can be
    // undone file for file (`converge update --rollback` restores the
    // newest update's snapshot; an init's stays on disk to copy back).
    let mut snapshot = Snapshot::begin("init");
    for tool in &chosen {
        if let Some(snapshot) = snapshot.as_mut() {
            for path in tool.artifacts() {
                snapshot.keep(&path)?;
            }
        }
        // One group per tool: what was written, the MCP question, and
        // whatever that tool still needs a human for — together, so the
        // Codex trust step is read next to the Codex hooks it gates.
        let label = tool.label();
        println!(
            "\n── {label} {}",
            "─".repeat(46usize.saturating_sub(label.len()))
        );

        let installed = tool.install(&exe)?;
        if installed.changed.is_empty() {
            println!(
                "  ✓ {}: already installed ({})",
                installed.noun,
                installed.path.display()
            );
        } else {
            println!(
                "  ✓ {}: installed {} ({})",
                installed.noun,
                installed.changed.join(", "),
                installed.path.display()
            );
        }

        // A registration bakes in the server URL and bearer, so a forced
        // reinit must replace it, not keep it.
        let registered = tool.mcp_registered();
        if registered && !force {
            println!("  ✓ mcp: `converge` server already registered");
        } else if confirm(&format!("  register the MCP server with {label}?"), true)? {
            if registered {
                tool.mcp_unregister()?;
            }
            tool.mcp_register(&config)?;
            println!("  ✓ mcp: registered {}/mcp as `converge`", config.server);
        } else {
            println!(
                "  skipped — register later with:\n    {}",
                tool.mcp_manual_hint(&config)
            );
        }

        for note in tool.notes(&config) {
            println!("  → {note}");
        }
    }

    if let Some(dir) = snapshot.map(Snapshot::finish).transpose()?.flatten() {
        println!("\nbackup of the files above: {}", dir.display());
    }

    let labels: Vec<_> = chosen.iter().map(|h| h.label()).collect();
    let labels = match labels.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => labels.join(""),
    };
    println!(
        "\ndone. Open any repository in {labels} — the session will \
         suggest a project binding (or run `converge project init` yourself)."
    );
    Ok(())
}

/// Which agent tools to wire. `--harness` decides outright. On a
/// terminal, a picker pre-checked with what is installed — tools that
/// were not found stay listed, unchecked, for the install `detect`
/// missed. On a pipe there is no one to ask: everything found gets
/// wired, which is what scripts and the e2e harness depend on.
fn chosen(only: &[Kind], config: &Config) -> Result<Option<Vec<&'static dyn Harness>>> {
    if !only.is_empty() {
        return Ok(Some(only.iter().map(|kind| kind.harness()).collect()));
    }
    let all: Vec<_> = harness::all().collect();
    let found: Vec<bool> = all.iter().map(|tool| tool.detect()).collect();

    if interactive() {
        let items: Vec<String> = all
            .iter()
            .zip(&found)
            .map(|(tool, found)| {
                if *found {
                    tool.label().to_string()
                } else {
                    format!("{} (not found)", tool.label())
                }
            })
            .collect();
        let theme = dialoguer::theme::ColorfulTheme::default();
        let picks = dialoguer::MultiSelect::with_theme(&theme)
            .with_prompt("integrate with")
            .items(&items)
            .defaults(&found)
            .interact()?;
        if picks.is_empty() {
            println!(
                "nothing selected — re-run `converge init` when you want to wire an agent tool."
            );
            return Ok(None);
        }
        return Ok(Some(picks.into_iter().map(|i| all[i]).collect()));
    }

    let present: Vec<_> = all
        .iter()
        .zip(&found)
        .filter(|(_, found)| **found)
        .map(|(tool, _)| *tool)
        .collect();
    if present.is_empty() {
        let known: Vec<_> = all.iter().map(|tool| tool.label()).collect();
        println!(
            "\nno supported agent tool found (looked for {}). Install one, \
             then re-run `converge init`; anything else: wire the four hook \
             commands (`converge hook inject|ctx|mark|sync`) and add the MCP \
             server {}/mcp manually.",
            known.join(", "),
            config.server
        );
        return Ok(None);
    }
    Ok(Some(present))
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
            Ok(token) => {
                return stored(Config {
                    server,
                    token,
                    auto_update: true,
                })
                .await;
            }
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
    stored(Config {
        server,
        token,
        auto_update: true,
    })
    .await
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

// ─── repair after an update ──────────────────────────────────────────────────

/// What an update refreshed, for the terminal now and for the next
/// session start (the automatic update runs with no terminal at all).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub from: String,
    pub to: String,
    pub backup: Option<PathBuf>,
    pub refreshed: Vec<Refreshed>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Refreshed {
    pub label: String,
    pub changed: Vec<String>,
    pub todo: Option<String>,
}

impl Report {
    fn path() -> Option<PathBuf> {
        Some(crate::state::dir()?.join("update-report.json"))
    }

    fn save(&self) -> Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("write {}", path.display()))
    }

    /// The report left by the last update, taken exactly once: the
    /// session-start line that shows it is the only reader.
    pub fn take() -> Option<Self> {
        let path = Self::path()?;
        let report = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
        let _ = std::fs::remove_file(path);
        Some(report)
    }

    /// One line for a human: what changed and what is left to do; empty
    /// when nothing changed.
    pub fn line(&self) -> Option<String> {
        if self.refreshed.is_empty() {
            return None;
        }
        let parts: Vec<String> = self
            .refreshed
            .iter()
            .map(|r| match &r.todo {
                Some(todo) => format!("{}: {} — {todo}", r.label, r.changed.join(", ")),
                None => format!("{}: {}", r.label, r.changed.join(", ")),
            })
            .collect();
        Some(format!("refreshed {}", parts.join("; ")))
    }
}

/// After the binary changed: re-run the install for every tool that is
/// already wired here, so hooks the new version registers and the
/// plugin it embeds match it. Idempotent, needs no credentials, keeps a
/// snapshot first. Runs in the *new* binary — the old one cannot know
/// what the new one wants installed.
pub fn repair(exe: &str, from: &str) -> Result<Report> {
    repair_with(&harness::all().collect::<Vec<_>>(), exe, from)
}

pub fn repair_with(tools: &[&dyn Harness], exe: &str, from: &str) -> Result<Report> {
    let to = crate::skew::CLI;
    let mut snapshot = Snapshot::begin(&format!("update-{from}-{to}"));
    let mut report = Report {
        from: from.to_owned(),
        to: to.to_owned(),
        backup: None,
        refreshed: Vec::new(),
    };
    for tool in tools {
        if !tool.integrated(exe) {
            continue;
        }
        if let Some(snapshot) = snapshot.as_mut() {
            for path in tool.artifacts() {
                snapshot.keep(&path)?;
            }
        }
        let installed = tool.install(exe)?;
        if !installed.changed.is_empty() {
            report.refreshed.push(Refreshed {
                label: tool.label().to_owned(),
                changed: installed.changed,
                todo: tool.after_refresh().map(str::to_owned),
            });
        }
    }
    report.backup = snapshot.map(Snapshot::finish).transpose()?.flatten();
    report.save()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::Value;

    use super::*;
    use crate::harness::{Installed, Payload, Response, Transcript};
    use crate::transcript::Parsed;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tool whose install writes the binary path into one file and
    /// reports the change once.
    struct Fake {
        file: PathBuf,
        wired: bool,
    }

    impl Harness for Fake {
        fn label(&self) -> &'static str {
            "Fake"
        }
        fn detect(&self) -> bool {
            true
        }
        fn install(&self, exe: &str) -> Result<Installed> {
            let current = std::fs::read_to_string(&self.file).unwrap_or_default();
            let changed = if current == exe {
                Vec::new()
            } else {
                std::fs::create_dir_all(self.file.parent().unwrap())?;
                std::fs::write(&self.file, exe)?;
                vec!["hooks".to_string()]
            };
            Ok(Installed {
                noun: "hooks",
                changed,
                path: self.file.clone(),
            })
        }
        fn mcp_registered(&self) -> bool {
            true
        }
        fn mcp_register(&self, _: &Config) -> Result<()> {
            Ok(())
        }
        fn mcp_unregister(&self) -> Result<()> {
            Ok(())
        }
        fn mcp_manual_hint(&self, _: &Config) -> String {
            String::new()
        }
        fn parse(&self, _: &Value) -> Payload {
            Payload::default()
        }
        fn emit(&self, _: Response) -> Option<Value> {
            None
        }
        fn transcript(&self, _: &Transcript) -> Result<Parsed> {
            bail!("a fake keeps no transcript")
        }
        fn artifacts(&self) -> Vec<PathBuf> {
            vec![self.file.clone()]
        }
        fn integrated(&self, _: &str) -> bool {
            self.wired
        }
        fn after_refresh(&self) -> Option<&'static str> {
            Some("restart it")
        }
    }

    #[test]
    fn repair_refreshes_only_what_is_wired_and_keeps_a_snapshot() {
        let _env = crate::state::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = temp();
        // Snapshots and the report live under the state dir.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        let wired = Fake {
            file: dir.join("wired").join("hooks.json"),
            wired: true,
        };
        std::fs::create_dir_all(wired.file.parent().unwrap()).unwrap();
        std::fs::write(&wired.file, "/old/converge").unwrap();
        let stranger = Fake {
            file: dir.join("stranger").join("hooks.json"),
            wired: false,
        };

        let report = repair_with(&[&wired, &stranger], "/new/converge", "0.1.0").unwrap();
        assert_eq!(report.from, "0.1.0");
        assert_eq!(report.to, crate::skew::CLI);
        assert_eq!(report.refreshed.len(), 1, "{report:?}");
        assert_eq!(report.refreshed[0].label, "Fake");
        assert_eq!(report.refreshed[0].changed, ["hooks"]);
        assert_eq!(
            report.line().as_deref(),
            Some("refreshed Fake: hooks — restart it")
        );
        assert_eq!(
            std::fs::read_to_string(&wired.file).unwrap(),
            "/new/converge"
        );
        // A tool that was never wired is never touched.
        assert!(!stranger.file.exists());

        // The snapshot holds the old bytes; restoring it undoes the refresh.
        let backup = report.backup.clone().expect("a snapshot was kept");
        assert!(Path::new(&backup).join("manifest.json").exists());
        crate::backup::restore(&backup).unwrap();
        assert_eq!(
            std::fs::read_to_string(&wired.file).unwrap(),
            "/old/converge"
        );

        // The report is there for exactly one reader.
        assert!(Report::take().is_some());
        assert!(Report::take().is_none());

        unsafe { std::env::remove_var("XDG_STATE_HOME") };
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
