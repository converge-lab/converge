//! `converge` — the client-side integration for coding agents.
//!
//! This binary lives on the *developer's* machine, next to Claude Code;
//! the server runs elsewhere. The main per-repository path is in-session
//! (the POC flow): hooks surface project suggestions to the agent, the
//! human decides in conversation, and hooks materialize the `.converge`
//! marker. The commands here are the scaffolding around that:
//!
//! - `converge project init` — the manual fallback: bind, rebind, or
//!   disable a repository interactively from the terminal.
//! - hook entrypoints and the global setup wizard arrive in the next
//!   slices.
//!
//! Configuration: `~/.config/converge/cli.toml` (`server`, and `token` or
//! preferably `token_cmd`), overridable with `CONVERGE_SERVER` /
//! `CONVERGE_TOKEN`.

mod config;
mod hook;
mod init;
mod marker;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "converge", about = "Converge agent integration", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Per-repository binding (the manual path; sessions normally bind
    /// through the agent).
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Hook entrypoints — invoked by the agent tool, never interactively.
    #[command(subcommand)]
    Hook(HookCmd),
}

#[derive(Subcommand)]
enum HookCmd {
    /// SessionStart: emit the context block for the marker's state.
    Inject,
    /// PreToolUse (converge tools): merge cwd + git remote into the call.
    Ctx,
    /// PostToolUse (binding tools): write the marker from the response.
    Apply,
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Bind this repository to a converge project (writes `.converge` at
    /// the git root; commit it). Suggests existing projects or creates
    /// one — never binds silently.
    Init {
        /// Re-run the binding even when already bound or disabled.
        #[arg(long)]
        rebind: bool,
        /// Opt this repository out: the integration stays quiet here.
        /// Works offline (no server needed to say no).
        #[arg(long, conflicts_with = "rebind")]
        off: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Cmd::Project(ProjectCmd::Init { rebind, off }) => init::run(rebind, off).await,
        Cmd::Hook(HookCmd::Inject) => hook::inject().await,
        Cmd::Hook(HookCmd::Ctx) => hook::ctx(),
        Cmd::Hook(HookCmd::Apply) => hook::apply(),
    }
}
