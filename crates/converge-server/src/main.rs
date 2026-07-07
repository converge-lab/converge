//! Binary entry — the composition root: load config → init telemetry →
//! connect + migrate PostgreSQL → serve until SIGINT/SIGTERM.
//!
//! One subcommand: `converge-server token mint [label]` prints a fresh
//! bearer secret for the deployment user to **stdout** and exits. Host
//! access is the trust boundary (the same model as running the server) —
//! secrets never enter the service log, where collectors would keep them.

mod config;
mod telemetry;

use anyhow::Context;
use config::ConfigService;
use converge_server::{app, auth};
use converge_storage::{Identity, Storage};
use converge_storage_postgres::PgStorage;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ConfigService::load()
        .context("load configuration")?
        .config();

    // The deployment's single-user identity: provider `local`, keyed by
    // the configured handle. Real providers (GitHub OIDC) land beside it.
    let me = Identity {
        provider: "local".into(),
        subject: config.user.handle.clone(),
        handle: config.user.handle.clone(),
        name: config.user.name.clone(),
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("token"), Some("mint")) => {
            let store = PgStorage::connect(&config.database_url).await?;
            store.migrate().await?;
            let label = args.get(2).cloned().unwrap_or_else(|| "cli".into());
            return mint(&store, me, label).await;
        }
        (Some(_), _) => anyhow::bail!("unknown command (try `token mint [label]`)"),
        (None, _) => {}
    }

    let _guard = telemetry::init(&config.log)?;
    info!(sources = ?config.sources, "configuration layers (weakest first, env on top)");

    let store = PgStorage::connect(&config.database_url).await?;
    store.migrate().await?;
    auth::hint(&store, me.clone()).await?;

    if let Some(assets) = &config.web.assets {
        info!(assets = %assets.display(), "serving web assets");
    }
    let listener = TcpListener::bind(config.listen).await?;
    info!(listen = %config.listen, "converge-server listening");
    axum::serve(listener, app(store, me, config.web.assets.as_deref()))
        .with_graceful_shutdown(shutdown())
        .await?;
    info!("shut down cleanly");
    Ok(())
}

/// `token mint [label]`: log the deployment user in, mint a bearer secret,
/// print it once to stdout.
async fn mint<S: Storage>(store: &S, me: Identity, label: String) -> anyhow::Result<()> {
    let user = store.user_login(me).await?;
    let secret = auth::mint();
    store.token_add(user, label, auth::hash(&secret)).await?;
    println!("{secret}");
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
