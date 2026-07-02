//! Binary entry — the composition root: load config → init telemetry →
//! connect + migrate PostgreSQL → serve until SIGINT/SIGTERM.

mod config;
mod telemetry;

use anyhow::Context;
use config::ConfigService;
use converge_server::app;
use converge_storage_postgres::PgStorage;
use tokio::net::TcpListener;
use tokio::signal;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ConfigService::load()
        .context("load configuration")?
        .config();
    let _guard = telemetry::init(&config.log)?;
    info!(sources = ?config.sources, "configuration layers (weakest first, env on top)");

    let store = PgStorage::connect(&config.database_url).await?;
    store.migrate().await?;

    let listener = TcpListener::bind(config.listen).await?;
    info!(listen = %config.listen, "converge-server listening");
    axum::serve(listener, app(store))
        .with_graceful_shutdown(shutdown())
        .await?;
    info!("shut down cleanly");
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
