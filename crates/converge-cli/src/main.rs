//! `converge` — the client-side integration for coding agents.
//!
//! This binary lives on the *developer's* machine, next to the agent
//! tool it integrates with; the server runs elsewhere. The main
//! per-repository path is in-session (the POC flow): hooks surface
//! project suggestions to the agent, the human decides in conversation,
//! and hooks materialize the `.converge` marker. The commands here are
//! the scaffolding around that:
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
mod device;
mod harness;
mod hook;
mod marker;
mod project;
mod setup;
mod skew;
mod transcript;
mod update;
mod watermark;

use clap::{Args, Parser, Subcommand};

use crate::harness::Kind;

#[derive(Parser)]
#[command(name = "converge", version, about = "Converge agent integration", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One-time machine setup: credentials, agent-tool integration
    /// (hooks + MCP). Safe to re-run.
    Init {
        /// Redo credentials and MCP registration even when the stored
        /// ones work — switch servers, or re-pair as someone else.
        #[arg(long)]
        force: bool,
    },
    /// Self-update from a signed release (or roll back to the kept
    /// previous binary).
    Update {
        /// A release tag (default: the latest).
        #[arg(long)]
        version: Option<String>,
        /// A local release directory (closed contours): artifact +
        /// SHA256SUMS + SHA256SUMS.minisig, verified the same way.
        #[arg(long, conflicts_with = "version")]
        from: Option<std::path::PathBuf>,
        /// Swap back to the previous binary.
        #[arg(long, conflicts_with_all = ["version", "from"])]
        rollback: bool,
        /// Reinstall even when already at the target version.
        #[arg(long)]
        force: bool,
    },
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
    /// Session start: emit the context block for the marker's state.
    Inject(Caller),
    /// Pre-tool (converge tools): merge cwd + git remote into the call.
    Ctx(Caller),
    /// Post-tool (binding tools): write the marker from the response.
    Mark(Caller),
    /// Session end: push new transcript turns into the evidence layer.
    Sync(Caller),
}

/// Which agent tool is calling — it decides how the payload is read and
/// how the answer is shaped. `converge init` writes the flag into the
/// command it registers, so a hook never guesses; the default keeps
/// every already-installed Claude Code command working unflagged.
#[derive(Args)]
struct Caller {
    #[arg(long = "harness", value_enum, default_value_t = Kind::Claude)]
    kind: Kind,
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
        Cmd::Init { force } => setup::run(force).await,
        Cmd::Update {
            version,
            from,
            rollback,
            force,
        } => update::run(version, from, rollback, force).await,
        Cmd::Project(ProjectCmd::Init { rebind, off }) => project::run(rebind, off).await,
        Cmd::Hook(HookCmd::Inject(c)) => hook::inject(c.kind).await,
        Cmd::Hook(HookCmd::Ctx(c)) => hook::ctx(c.kind),
        Cmd::Hook(HookCmd::Mark(c)) => hook::mark(c.kind),
        Cmd::Hook(HookCmd::Sync(c)) => hook::sync(c.kind).await,
    }
}
