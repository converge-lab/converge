//! Dev tooling for the Converge workspace.
//!
//! Owns the local Postgres lifecycle via testcontainers, so there's no
//! committed compose file and no standing database to manage. Run via the
//! `cargo xtask` alias (see `.cargo/config.toml`); `cargo xtask --help` for
//! the commands. All need only Docker; `prepare` additionally shells
//! `cargo sqlx` (`cargo install sqlx-cli`), `dev --web` shells `trunk`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

/// Postgres 16 — the schema uses `unique nulls not distinct` (Postgres 15+).
const PG_TAG: &str = "16-alpine";

#[derive(Parser)]
#[command(about = "Converge dev tooling", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Boot Postgres, apply migrations, print DATABASE_URL, hold until Ctrl-C.
    Db,
    /// The full stack: boot Postgres, migrate, mint a dev token, run the
    /// server until Ctrl-C. The database is ephemeral — it dies with the
    /// command.
    Dev {
        /// Also `trunk build` the web app and serve it same-origin.
        #[arg(long)]
        web: bool,
    },
    /// Boot an ephemeral Postgres, migrate, and regenerate the sqlx cache.
    Prepare {
        /// Verify the committed .sqlx/ cache is current instead of writing it (CI).
        #[arg(long)]
        check: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Db => db().await,
        Cmd::Dev { web } => dev(web).await,
        Cmd::Prepare { check } => prepare(check).await,
    }
}

/// Boot Postgres, migrate, and hold it open until Ctrl-C.
async fn db() -> Result<()> {
    let (node, url) = start_pg().await?;
    println!("DATABASE_URL={url}");
    println!("ready — Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    drop(node); // remove the container
    Ok(())
}

/// The full dev stack: Postgres + migrations, a freshly minted bearer
/// token (printed below — the terminal, not the log, is where secrets
/// belong), and the server in the foreground. A fixed dev session secret
/// keeps browser logins across server restarts within one `dev` run.
async fn dev(web: bool) -> Result<()> {
    let (node, url) = start_pg().await?;

    let assets = if web {
        let status = Command::new("trunk")
            .args(["build", "--features", "api"])
            .current_dir(workspace_root().join("crates/converge-web"))
            .status()
            .context("running `trunk build` (is trunk installed?)")?;
        if !status.success() {
            bail!("`trunk build` failed with {status}");
        }
        Some(workspace_root().join("crates/converge-web/dist"))
    } else {
        None
    };

    // Mint through the server binary so the hashing convention has exactly
    // one home; `cargo run` also pre-builds the binary the stack is about
    // to run.
    let minted = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "converge-server",
            "--",
            "token",
            "mint",
            "dev",
        ])
        .env("CONVERGE_DATABASE_URL", &url)
        .current_dir(workspace_root())
        .output()
        .context("minting the dev token")?;
    if !minted.status.success() {
        bail!(
            "`converge-server token mint` failed:\n{}",
            String::from_utf8_lossy(&minted.stderr)
        );
    }
    let token = String::from_utf8(minted.stdout)?.trim().to_string();

    println!("DATABASE_URL={url}");
    println!("token: {token}");
    println!(
        "web:   http://127.0.0.1:8080/ ({})",
        match &assets {
            Some(_) => "serving crates/converge-web/dist",
            None => "no assets; use `--web`, or `trunk serve` for live reload",
        }
    );
    println!("mcp:   http://127.0.0.1:8080/mcp (Authorization: Bearer <token>)");
    println!("ready — Ctrl-C to stop");

    // Ctrl-C reaches the whole foreground group; registering a handler
    // keeps xtask alive to wait out the server's graceful shutdown and
    // then remove the container.
    tokio::spawn(async {
        loop {
            let _ = tokio::signal::ctrl_c().await;
        }
    });
    let mut server = tokio::process::Command::new("cargo");
    server
        .args(["run", "-q", "-p", "converge-server"])
        .env("CONVERGE_DATABASE_URL", &url)
        .env("CONVERGE_AUTH__SESSION_SECRET", "converge-dev")
        .current_dir(workspace_root());
    if let Some(dist) = &assets {
        server.env("CONVERGE_WEB__ASSETS", dist);
    }
    let status = server
        .spawn()
        .context("starting converge-server")?
        .wait()
        .await?;
    drop(node); // remove the container
    if !status.success() {
        bail!("converge-server exited with {status}");
    }
    Ok(())
}

/// Boot an ephemeral Postgres, migrate, and (re)generate or check the sqlx
/// offline cache via `cargo sqlx prepare`.
async fn prepare(check: bool) -> Result<()> {
    let (_node, url) = start_pg().await?;
    let mut args = vec!["sqlx", "prepare", "--workspace"];
    if check {
        args.push("--check");
    }
    let status = Command::new("cargo")
        .args(&args)
        .env("DATABASE_URL", &url)
        .current_dir(workspace_root())
        .status()
        .context("running `cargo sqlx prepare` (is sqlx-cli installed?)")?;
    if !status.success() {
        bail!("`cargo sqlx prepare` failed with {status}");
    }
    println!(
        "{}",
        if check {
            ".sqlx/ is current"
        } else {
            "regenerated .sqlx/"
        }
    );
    Ok(())
}

/// Start a Postgres container and apply the migrations, returning the handle
/// (dropping it removes the container) and the connection URL.
async fn start_pg() -> Result<(ContainerAsync<Postgres>, String)> {
    let node = Postgres::default()
        .with_tag(PG_TAG)
        .start()
        .await
        .context("starting the Postgres container (is Docker running?)")?;
    let port = node.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let pool = PgPool::connect(&url)
        .await
        .context("connecting to the dev Postgres")?;
    sqlx::migrate!("../converge-storage-postgres/migrations")
        .run(&pool)
        .await
        .context("applying migrations")?;
    Ok((node, url))
}

/// The workspace root: `.../crates/xtask` → `...`.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("xtask lives at crates/xtask")
        .to_path_buf()
}
