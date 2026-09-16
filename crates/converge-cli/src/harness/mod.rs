//! The agent-tool abstraction.
//!
//! Converge integrates with several coding agents. They agree on *what*
//! the integration needs — a context block at session start, the working
//! directory merged into converge tool calls, the marker written after a
//! binding, the conversation pushed as evidence — and disagree on
//! everything else: the field names, the file that wires it up, and
//! whether the mechanism is a subprocess at all (opencode runs an
//! in-process TypeScript plugin, not a hook command).
//!
//! So each agent tool gets a [`Harness`] owning three edges:
//!
//! - **install** — the artifact it writes (a hooks file, a plugin, a CLI
//!   call), and what the human still has to do afterwards;
//! - **parse** — its invocation payload into our [`Payload`];
//! - **emit** — our [`Response`] into its output contract.
//!
//! The entrypoints in [`crate::hook`] work on `Payload`/`Response` only
//! and know none of it. Which harness is calling is never guessed:
//! installation bakes the choice into the command it writes.

mod claude;
mod codex;
mod hooks_file;
mod wire;

use std::path::PathBuf;

use anyhow::Result;
use clap::ValueEnum;
use serde_json::Value;

use crate::config::Config;
use crate::transcript::Parsed;

/// Every agent tool Converge knows how to wire itself into.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    /// Claude Code — the default, because it is what every already
    /// installed hook command omits the flag for.
    #[default]
    Claude,
    /// Codex CLI.
    Codex,
}

static CLAUDE: claude::ClaudeCode = claude::ClaudeCode;
static CODEX: codex::CodexCli = codex::CodexCli;

impl Kind {
    pub fn harness(self) -> &'static dyn Harness {
        match self {
            Kind::Claude => &CLAUDE,
            Kind::Codex => &CODEX,
        }
    }
}

/// Every harness, in the order `converge init` considers them. Driven
/// off the enum itself, so adding a `Kind` is the only place a new agent
/// tool has to be listed.
pub fn all() -> impl Iterator<Item = &'static dyn Harness> {
    Kind::value_variants().iter().copied().map(Kind::harness)
}

/// The harnesses actually present on this machine.
pub fn present() -> Vec<&'static dyn Harness> {
    all().filter(|harness| harness.detect()).collect()
}

/// One invocation, normalized. Whatever a harness hands us, only these
/// four things mean anything to Converge.
#[derive(Debug, Default)]
pub struct Payload {
    /// The working directory the session runs in — what resolves the
    /// marker, and therefore the project.
    pub cwd: PathBuf,
    /// Pre-tool: the arguments the model chose.
    pub tool_input: Value,
    /// Post-tool: what the tool answered.
    pub tool_response: Value,
    /// Session end: where the conversation can be read back, if at all.
    pub transcript: Option<Transcript>,
}

/// Where a harness keeps the conversation. Deliberately not a path:
/// opencode holds sessions in SQLite and hands out an id instead.
#[derive(Debug, Clone)]
pub enum Transcript {
    File(PathBuf),
}

impl Transcript {
    /// The sync watermark key. Stable across runs — it is what decides
    /// which turns were already sent.
    pub fn key(&self) -> String {
        match self {
            Transcript::File(path) => path.to_string_lossy().into_owned(),
        }
    }
}

/// What an entrypoint decided to say back, before any harness dialect.
pub enum Response {
    /// Session start: the context block, plus the one line a human sees.
    Inject { context: String, system: String },
    /// Pre-tool: the tool arguments, enriched.
    Ctx { tool_input: Value },
    /// A visible line and nothing else.
    Notice { system: String },
    /// Nothing to say — emit stays quiet rather than printing `null`.
    Silent,
}

/// What [`Harness::install`] did, for the line `converge init` prints.
pub struct Installed {
    /// What the artifact is called in that line: `hooks`, `plugin`, …
    pub noun: &'static str,
    /// What changed, empty when everything was already in place.
    pub changed: Vec<String>,
    /// Where it was written.
    pub path: PathBuf,
}

pub trait Harness: Sync {
    /// How the tool calls itself, wherever a human reads it.
    fn label(&self) -> &'static str;

    /// Is this tool on the machine?
    fn detect(&self) -> bool;

    /// Wire the integration in. Idempotent: every step detects "already
    /// done" and moves on, so re-running after an upgrade or a moved
    /// binary is the repair path. `exe` is this binary's absolute path.
    fn install(&self, exe: &str) -> Result<Installed>;

    /// Anything the human still has to do by hand afterwards.
    fn notes(&self, _config: &Config) -> Vec<String> {
        Vec::new()
    }

    /// Is the converge MCP server already known to this tool?
    fn mcp_registered(&self) -> bool;

    /// Register it. A registration bakes in the server URL and bearer,
    /// so a forced reinit replaces rather than keeps one.
    fn mcp_register(&self, config: &Config) -> Result<()>;

    /// Drop an existing registration.
    fn mcp_unregister(&self) -> Result<()>;

    /// What to tell someone who declined the automatic registration.
    fn mcp_manual_hint(&self, config: &Config) -> String;

    /// This tool's invocation payload → ours.
    fn parse(&self, raw: &Value) -> Payload;

    /// Ours → this tool's output contract. `None` prints nothing.
    fn emit(&self, response: Response) -> Option<Value>;

    /// Read back a conversation this tool recorded.
    fn transcript(&self, at: &Transcript) -> Result<Parsed>;
}

/// The `cwd` fallback every harness shares: the field if it sent one,
/// otherwise wherever the hook process happens to be.
pub(crate) fn cwd_or_current(field: Option<&str>) -> PathBuf {
    field
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}
