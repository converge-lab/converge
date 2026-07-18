//! Binary entry — the composition root: load config → init telemetry →
//! connect + migrate PostgreSQL → serve until SIGINT/SIGTERM.
//!
//! The `token` subcommands (`mint`/`list`/`revoke`) administer bearer
//! tokens from the host and exit; secrets print to **stdout**, never the
//! service log, where collectors would keep them. Host access is the
//! trust boundary (the same model as running the server), so `--user` may
//! target any local-provider user — that's how a closed-contour operator
//! provisions teammates without an identity provider. Users manage their
//! own tokens over `/api/v1/tokens`.

mod config;
mod telemetry;

use anyhow::Context;
use clap::{Parser, Subcommand};
use config::ConfigService;
use converge_server::auth::Sessions;
use converge_server::{app, auth};
use converge_storage::{Identity, Pagination, Storage, TokenId};
use converge_storage_postgres::PgStorage;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::info;

#[derive(Parser)]
#[command(about = "The Converge server", long_about = None)]
struct Cli {
    /// With no command: serve.
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Administer bearer tokens (host trust; secrets print to stdout).
    #[command(subcommand)]
    Token(TokenCmd),
    /// The expert-model layer (`[expert]` config).
    #[command(subcommand)]
    Expert(ExpertCmd),
}

#[derive(Subcommand)]
enum ExpertCmd {
    /// Send a canned prompt through each configured job's model and
    /// report the reply and latency. No database needed.
    Check {
        /// Check this job only.
        job: Option<String>,
    },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Mint a token and print the secret — once.
    Mint {
        /// What the token is for ("laptop", "ci", …).
        #[arg(default_value = "cli")]
        label: String,
        /// Local-provider handle to mint for; creates the user if absent.
        /// Defaults to the deployment user from `[user]` config.
        #[arg(long)]
        user: Option<String>,
    },
    /// List a user's tokens (ids, labels, creation times — no secrets).
    List {
        /// Local-provider handle; defaults to the deployment user.
        #[arg(long)]
        user: Option<String>,
    },
    /// Revoke a token by id — the credential dies immediately.
    Revoke {
        /// The token id (see `token list`).
        id: TokenId,
        /// Local-provider handle owning the token; defaults to the
        /// deployment user.
        #[arg(long)]
        user: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = ConfigService::load()
        .context("load configuration")?
        .config();

    // The deployment's single-user identity: provider `local`, keyed by
    // the configured handle. Real providers (OIDC) land beside it.
    let me = Identity {
        provider: "local".into(),
        subject: config.user.handle.clone(),
        handle: config.user.handle.clone(),
        name: config.user.name.clone(),
    };

    if let Some(Cmd::Expert(ExpertCmd::Check { job })) = &cli.command {
        return check(&config.expert, job.as_deref()).await;
    }

    if let Some(Cmd::Token(cmd)) = cli.command {
        let store = PgStorage::connect(&config.database_url).await?;
        store.migrate().await?;
        return token(&store, me, cmd).await;
    }

    let _guard = telemetry::init(&config.log)?;
    info!(sources = ?config.sources, "configuration layers (weakest first, env on top)");

    // Build the expert registry even though nothing consumes it yet: a
    // bad binding or key command must fail the boot, not the first
    // signal. The signal pipeline picks it up when it lands.
    let expert = converge_expert::Registry::new(&config.expert)
        .context("build the expert registry ([expert] config)")?;
    for (job, client) in expert.jobs() {
        info!(job, model = client.describe(), "expert job configured");
    }

    let store = PgStorage::connect(&config.database_url).await?;
    store.migrate().await?;
    if store.ensure_default_workspace().await? {
        info!("created the default personal workspace (My workspace)");
    }
    auth::hint(&store, me.clone()).await?;

    if let Some(assets) = &config.web.assets {
        info!(assets = %assets.display(), "serving web assets");
    }
    let sessions = Sessions::new(config.auth.session_secret.as_deref());
    let oidc = config
        .auth
        .oidc
        .clone()
        .map(converge_server::oidc::Oidc::new);
    if let Some(oidc) = &oidc {
        info!(provider = %oidc.label(), "identity-provider sign-in enabled");
    }
    let listener = TcpListener::bind(config.listen).await?;
    info!(listen = %config.listen, "converge-server listening");
    axum::serve(
        listener,
        app(
            store,
            me,
            sessions,
            oidc,
            config.auth.public_url.clone(),
            config.web.assets.as_deref(),
        ),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    info!("shut down cleanly");
    Ok(())
}

/// `expert check`: exercise each configured job's model with a canned
/// prompt — config validation, connectivity, and credentials in one
/// command. Replies print to stdout; exits nonzero if any job fails.
async fn check(config: &converge_expert::Config, only: Option<&str>) -> anyhow::Result<()> {
    let registry = converge_expert::Registry::new(config)
        .context("build the expert registry ([expert] config)")?;
    if registry.is_empty() {
        println!("no expert jobs configured ([expert.jobs] is empty) — nothing to check");
        return Ok(());
    }
    if let Some(job) = only
        && registry.job(job).is_none()
    {
        let jobs: Vec<_> = registry.jobs().map(|(name, _)| name).collect();
        anyhow::bail!(
            "job \"{job}\" is not configured (configured: {})",
            jobs.join(", ")
        );
    }

    let mut failed = false;
    for (job, client) in registry.jobs() {
        if only.is_some_and(|o| o != job) {
            continue;
        }
        let start = std::time::Instant::now();
        match client
            .reply(
                "You are the health check for converge's expert layer. \
                 Reply with the single word: ok",
                "healthcheck",
            )
            .await
        {
            Ok(reply) => {
                let reply = reply.lines().next().unwrap_or_default();
                println!(
                    "{job}: ok — {} answered {reply:?} in {:.1?}",
                    client.describe(),
                    start.elapsed()
                );
            }
            Err(error) => {
                failed = true;
                println!("{job}: FAILED — {}: {error}", client.describe());
            }
        }
    }
    if failed {
        anyhow::bail!("expert check failed");
    }
    Ok(())
}

/// Run one `token` subcommand against the store and exit.
async fn token<S: Storage>(store: &S, me: Identity, cmd: TokenCmd) -> anyhow::Result<()> {
    // `--user` targets (or creates) a local-provider identity — the
    // closed-contour provisioning path; the handle doubles as the display
    // name until the person's first real login refreshes it.
    let resolve = |user: Option<String>| match user {
        None => me,
        Some(handle) => Identity {
            provider: "local".into(),
            subject: handle.clone(),
            name: handle.clone(),
            handle,
        },
    };
    match cmd {
        TokenCmd::Mint { label, user } => {
            let user = store.user_login(resolve(user)).await?;
            let secret = auth::mint();
            store.token_add(user, label, auth::hash(&secret)).await?;
            println!("{secret}");
        }
        TokenCmd::List { user } => {
            let user = store.user_login(resolve(user)).await?;
            for token in store.token_list(user, Pagination::default()).await? {
                println!("{}\t{}\t{}", token.id, token.created_at, token.label);
            }
        }
        TokenCmd::Revoke { id, user } => {
            let user = store.user_login(resolve(user)).await?;
            store.token_revoke(user, id).await?;
            println!("revoked {id}");
        }
    }
    Ok(())
}

/// Resolves on SIGINT (ctrl-c) or SIGTERM (systemd stop).
async fn shutdown() {
    let ctrl_c = async { signal::ctrl_c().await.expect("install ctrl-c handler") };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
