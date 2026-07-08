//! Daemon configuration: layered files, merged per key, env on top.
//!
//! Layers, weakest first — later layers override earlier ones **per key**
//! (git-config style), and every file is optional except an explicitly
//! named `CONVERGE_CONFIG`, which must exist (a misspelled path should fail
//! loudly, not fall back):
//!
//! 1. `/etc/converge/config.toml` (system)
//! 2. `$XDG_CONFIG_HOME/converge/config.toml`, default `~/.config/…` (user)
//! 3. `./converge.toml` (working directory — dev convenience)
//! 4. `$CONVERGE_CONFIG` (explicit, required when set)
//! 5. `CONVERGE_*` environment variables — nested keys use `__`
//!    (`CONVERGE_LOG__FILTER` → `log.filter`)
//!
//! Loading happens before tracing is initialized (the filter lives here), so
//! `load` stays silent; the merged [`Config::sources`] are logged by `main`.

use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use config::{ConfigError, Environment, File, FileFormat};
use serde::Deserialize;

/// Owns the loaded configuration for the process lifetime — the one place
/// lifecycle concerns (a future SIGHUP reload via `ArcSwap`) will live.
/// Consumers hold the `Arc<Config>` snapshot from [`ConfigService::config`];
/// that contract survives the reload upgrade unchanged
/// (`ArcSwap::load_full` returns the same shape), so call sites never churn.
/// (Named in full — a bare `Service` would collide with `tower::Service`,
/// which axum middleware keeps in scope.)
pub struct ConfigService {
    current: Arc<Config>,
}

impl ConfigService {
    /// Load the layered configuration (see the module docs for the order).
    pub fn load() -> Result<Self, ConfigError> {
        Ok(Self {
            current: Arc::new(Config::load()?),
        })
    }

    /// The current configuration snapshot.
    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.current)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Postgres connection string. Required — the only setting with no
    /// sensible default.
    pub database_url: String,
    /// Listen address, `host:port`.
    #[serde(default = "listen")]
    pub listen: SocketAddr,
    /// Logging (`[log]` table).
    #[serde(default)]
    pub log: Log,
    /// The deployment's user (`[user]` table) — what `/api/v1/users/me`
    /// resolves to in single-user mode, until real auth lands.
    #[serde(default)]
    pub user: User,
    /// Web UI assets (`[web]` table). Unset in dev — `trunk serve` fronts
    /// the API there; set to the trunk `dist/` directory in deployments so
    /// the server serves the app same-origin.
    #[serde(default)]
    pub web: Web,
    /// Authentication (`[auth]` table).
    #[serde(default)]
    pub auth: Auth,
    /// The config files that existed and merged, weakest first — provenance
    /// for the startup log, not a setting.
    #[serde(skip)]
    pub sources: Vec<String>,
}

/// The single-user identity (`handle` is the natural key; `name` display).
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    #[serde(default = "handle")]
    pub handle: String,
    #[serde(default = "display")]
    pub name: String,
}

impl Default for User {
    fn default() -> Self {
        Self {
            handle: handle(),
            name: display(),
        }
    }
}

fn handle() -> String {
    "admin".into()
}

fn display() -> String {
    "Admin".into()
}

/// Web-asset serving (`assets` = a trunk `dist/` directory).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Web {
    #[serde(default)]
    pub assets: Option<std::path::PathBuf>,
}

/// Authentication.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Auth {
    /// Key for signing browser-session JWTs. Unset → random per boot
    /// (sessions reset on restart). Set it (e.g. `openssl rand -hex 32`)
    /// to keep sessions across restarts.
    #[serde(default)]
    pub session_secret: Option<String>,
    /// Identity-provider sign-in (`[auth.oidc]`, see
    /// [`converge_server::oidc::Settings`]). Absent → token-paste only;
    /// the auth core never needs egress.
    #[serde(default)]
    pub oidc: Option<converge_server::oidc::Settings>,
    /// This deployment's external origin, used as the OAuth issuer for
    /// MCP connectors. Absent → derived per-request from the `Host`
    /// header (fine for dev; set it behind a proxy).
    #[serde(default)]
    pub public_url: Option<String>,
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Log {
    /// Tracing filter directives, e.g. `info` or
    /// `converge_server=debug,info`.
    #[serde(default = "filter")]
    pub filter: String,
}

impl Default for Log {
    fn default() -> Self {
        Self { filter: filter() }
    }
}

fn filter() -> String {
    "info".into()
}

fn listen() -> SocketAddr {
    "127.0.0.1:8080"
        .parse()
        .expect("valid default listen address")
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let mut files = vec!["/etc/converge/config.toml".to_owned()];
        if let Some(user) = user_file() {
            files.push(user);
        }
        files.push("converge.toml".to_owned());

        let mut builder = config::Config::builder();
        for file in &files {
            builder = builder.add_source(File::new(file, FileFormat::Toml).required(false));
        }
        if let Ok(explicit) = env::var("CONVERGE_CONFIG") {
            builder = builder.add_source(File::new(&explicit, FileFormat::Toml).required(true));
            files.push(explicit);
        }

        let mut config: Self = builder
            .add_source(
                Environment::with_prefix("CONVERGE")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?
            .try_deserialize()?;
        config.sources = files
            .into_iter()
            .filter(|f| Path::new(f).exists())
            .collect();
        Ok(config)
    }
}

/// `$XDG_CONFIG_HOME/converge/config.toml`, defaulting the base to
/// `~/.config`. `None` when neither variable is set (e.g. a bare container).
fn user_file() -> Option<String> {
    let base = env::var("XDG_CONFIG_HOME")
        .or_else(|_| env::var("HOME").map(|home| format!("{home}/.config")))
        .ok()?;
    Some(format!("{base}/converge/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_layers(layers: &[&str]) -> Result<Config, ConfigError> {
        let mut builder = config::Config::builder();
        for toml in layers {
            builder = builder.add_source(File::from_str(toml, FileFormat::Toml));
        }
        builder.build()?.try_deserialize()
    }

    #[test]
    fn file_with_defaults() {
        let cfg = from_layers(&[r#"database_url = "postgres://x""#]).unwrap();
        assert_eq!(cfg.database_url, "postgres://x");
        assert_eq!(cfg.listen, listen());
        assert_eq!(cfg.log.filter, "info");
    }

    #[test]
    fn later_layer_overrides_per_key() {
        let cfg = from_layers(&[
            r#"
            database_url = "postgres://system"
            listen = "0.0.0.0:9000"

            [log]
            filter = "warn"
            "#,
            r#"
            [log]
            filter = "converge_server=debug,info"
            "#,
        ])
        .unwrap();
        // The user layer overrode `log.filter`; the rest fell through.
        assert_eq!(cfg.database_url, "postgres://system");
        assert_eq!(cfg.listen, "0.0.0.0:9000".parse::<SocketAddr>().unwrap());
        assert_eq!(cfg.log.filter, "converge_server=debug,info");
    }

    #[test]
    fn missing_database_url_fails() {
        assert!(from_layers(&[""]).is_err());
    }
}
