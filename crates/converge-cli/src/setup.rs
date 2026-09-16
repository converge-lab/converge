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

use anyhow::{Context, Result, bail};

use crate::config::{self, Config};
use crate::{device, harness};

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
    // Every tool on the machine gets wired — people run more than one,
    // and a tool installed later is what `converge init` is re-run for.
    let present = harness::present();
    if present.is_empty() {
        let known: Vec<_> = harness::all().map(|h| h.label()).collect();
        println!(
            "\nno supported agent tool found (looked for {}). Install one, \
             then re-run `converge init`; anything else: wire the four hook \
             commands (`converge hook inject|ctx|mark|sync`) and add the MCP \
             server {}/mcp manually.",
            known.join(", "),
            config.server
        );
        return Ok(());
    }

    let exe = std::env::current_exe().context("resolve own path")?;
    let exe = exe.to_string_lossy();
    for tool in &present {
        let installed = tool.install(&exe)?;
        if installed.changed.is_empty() {
            println!(
                "✓ {}: already installed ({})",
                installed.noun,
                installed.path.display()
            );
        } else {
            println!(
                "✓ {}: installed {} ({})",
                installed.noun,
                installed.changed.join(", "),
                installed.path.display()
            );
        }

        // ── MCP registration ─────────────────────────────────────────
        // A registration bakes in the server URL and bearer, so a forced
        // reinit must replace it, not keep it.
        let registered = tool.mcp_registered();
        if registered && !force {
            println!("✓ mcp: `converge` server already registered");
        } else if confirm(
            &format!("register the MCP server with {}?", tool.label()),
            true,
        )? {
            if registered {
                tool.mcp_unregister()?;
            }
            tool.mcp_register(&config)?;
            println!("✓ mcp: registered {}/mcp as `converge`", config.server);
        } else {
            println!(
                "skipped — register later with:\n  {}",
                tool.mcp_manual_hint(&config)
            );
        }
    }

    let labels: Vec<_> = present.iter().map(|h| h.label()).collect();
    println!(
        "\ndone. Open any repository in {} — the session will \
         suggest a project binding (or run `converge project init` yourself).",
        labels.join(" or ")
    );
    for tool in &present {
        for note in tool.notes(&config) {
            println!("\n{note}");
        }
    }
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
