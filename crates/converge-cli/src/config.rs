//! CLI configuration: where the server is and how to authenticate.
//!
//! One file — `~/.config/converge/cli.toml` (`$XDG_CONFIG_HOME` honored)
//! — plus `CONVERGE_SERVER` / `CONVERGE_TOKEN` environment overrides. The
//! token is an ordinary bearer secret (mint one in the web UI's Settings,
//! or `converge-server token mint` on the server host). Prefer
//! `token_cmd` — a command that *prints* the secret (a password manager
//! call) — over `token` in plaintext; the command form wins when both are
//! set.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
struct File {
    server: Option<String>,
    token: Option<String>,
    token_cmd: Option<String>,
}

/// Resolved configuration: a server origin and a bearer secret.
pub struct Config {
    pub server: String,
    pub token: String,
}

impl Config {
    /// Load file + env; fail with instructions when either half is
    /// missing — the CLI is useless unconfigured, so say exactly what to
    /// do.
    pub fn load() -> Result<Self> {
        let file = match path() {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                toml::from_str::<File>(&text)
                    .with_context(|| format!("parse {}", path.display()))?
            }
            _ => File::default(),
        };

        let Some(server) = env::var("CONVERGE_SERVER").ok().or(file.server) else {
            bail!(
                "no server configured — set `server = \"https://…\"` in \
                 ~/.config/converge/cli.toml or CONVERGE_SERVER"
            );
        };
        let token = match env::var("CONVERGE_TOKEN").ok() {
            Some(token) => token,
            None => match (&file.token_cmd, &file.token) {
                (Some(cmd), _) => run(cmd)?,
                (None, Some(token)) => token.clone(),
                (None, None) => bail!(
                    "no token configured — mint one (web Settings, or \
                     `converge-server token mint` on the server host) and set \
                     `token_cmd`/`token` in ~/.config/converge/cli.toml or \
                     CONVERGE_TOKEN"
                ),
            },
        };
        Ok(Self {
            server: server.trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
        })
    }

    /// An authenticated API client for this configuration.
    pub fn client(&self) -> Result<converge_client::Client> {
        let base = self
            .server
            .parse()
            .with_context(|| format!("`{}` is not a URL", self.server))?;
        Ok(converge_client::Client::new(base).with_token(self.token.clone()))
    }
}

fn path() -> Option<PathBuf> {
    let base = env::var("XDG_CONFIG_HOME")
        .or_else(|_| env::var("HOME").map(|home| format!("{home}/.config")))
        .ok()?;
    Some(PathBuf::from(base).join("converge/cli.toml"))
}

/// Run a `token_cmd` and take its stdout as the secret.
fn run(cmd: &str) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("run token_cmd `{cmd}`"))?;
    if !output.status.success() {
        bail!("token_cmd `{cmd}` failed with {}", output.status);
    }
    Ok(String::from_utf8(output.stdout)
        .context("token_cmd printed non-UTF-8")?
        .trim()
        .to_string())
}
