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
mod cursor;
mod hooks_file;
mod json_file;
mod opencode;
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
    /// opencode.
    Opencode,
    /// Cursor.
    Cursor,
}

static CLAUDE: claude::ClaudeCode = claude::ClaudeCode;
static CODEX: codex::CodexCli = codex::CodexCli;
static OPENCODE: opencode::OpenCode = opencode::OpenCode;
static CURSOR: cursor::Cursor = cursor::Cursor;

impl Kind {
    pub fn harness(self) -> &'static dyn Harness {
        match self {
            Kind::Claude => &CLAUDE,
            Kind::Codex => &CODEX,
            Kind::Opencode => &OPENCODE,
            Kind::Cursor => &CURSOR,
        }
    }

    /// The value `--harness` takes for this kind: what `converge init`
    /// writes into hook commands, and what the server records a session
    /// under.
    pub fn flag(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::Opencode => "opencode",
            Kind::Cursor => "cursor",
        }
    }
}

/// Every harness, in the order `converge init` considers them. Driven
/// off the enum itself, so adding a `Kind` is the only place a new agent
/// tool has to be listed.
pub fn all() -> impl Iterator<Item = &'static dyn Harness> {
    Kind::value_variants().iter().copied().map(Kind::harness)
}

/// One invocation, normalized. Whatever a harness hands us, only these
/// four things mean anything to Converge.
#[derive(Debug, Default)]
pub struct Payload {
    /// The working directory the session runs in — what resolves the
    /// marker, and therefore the project.
    pub cwd: PathBuf,
    /// Which tool is being called, when the harness says. Present so the
    /// entrypoints can refuse to touch a tool that is not ours: a
    /// harness whose config cannot express "only converge's MCP tools"
    /// hands us every call instead, and rewriting an unrelated tool's
    /// arguments would be worse than doing nothing.
    pub tool_name: Option<String>,
    /// Pre-tool: the arguments the model chose.
    pub tool_input: Value,
    /// Post-tool: what the tool answered.
    pub tool_response: Value,
    /// Session end: where the conversation can be read back, if at all.
    pub transcript: Option<Transcript>,
    /// The harness's id for this conversation, when it says. Receipts
    /// are keyed by it: no session, nothing to draw a line for.
    pub session: Option<String>,
}

/// Where a harness keeps the conversation. Deliberately not a path:
/// opencode holds sessions in SQLite and hands out an id instead.
#[derive(Debug, Clone)]
pub enum Transcript {
    File(PathBuf),
    Session(String),
}

impl Transcript {
    /// The sync watermark key. Stable across runs — it is what decides
    /// which turns were already sent. Prefixed per variant so a session
    /// id can never collide with a path.
    pub fn key(&self) -> String {
        match self {
            Transcript::File(path) => path.to_string_lossy().into_owned(),
            Transcript::Session(id) => format!("session:{id}"),
        }
    }
}

/// What an entrypoint decided to say back, before any harness dialect.
pub enum Response {
    /// Session start: the context block, plus the one line a human sees.
    /// `sticky` says the block is reference material worth carrying in
    /// every request (a bound project's decision index) rather than
    /// instructions to give once (unbound, disabled, unreadable) — a
    /// harness that rebuilds its prompt per request needs the
    /// difference. `degraded` marks a fallback (cached or unavailable
    /// index) that such a harness should refresh sooner.
    Inject {
        context: String,
        system: String,
        sticky: bool,
        degraded: bool,
        /// The marker state the block describes: `bound`, `disabled`,
        /// `unbound` or `unreadable`. A harness that treats subagents
        /// differently needs it: they get the index, nothing else.
        state: &'static str,
    },
    /// Pre-tool: the tool arguments, enriched.
    Ctx { tool_input: Value },
    /// A visible line and nothing else.
    Notice { system: String },
    /// Post-tool: what `mark` did to the marker, and the line for it. A
    /// harness that keeps state of its own (opencode's shim) needs the
    /// effect, not just the prose: a session-scoped dismiss writes no
    /// marker but must still silence that session.
    Marked {
        effect: Effect,
        system: Option<String>,
    },
    /// Nothing to say — emit stays quiet rather than printing `null`.
    Silent,
}

/// What `hook mark` did with a binding tool's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Wrote a bound marker.
    Bound,
    /// Wrote a disabled marker.
    Disabled,
    /// The user declined for this session only; nothing on disk.
    DismissedSession,
    /// A skip or an unrecognised answer; nothing to do.
    Nothing,
    /// The marker could not be written.
    Failed,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Bound => "bound",
            Effect::Disabled => "disabled",
            Effect::DismissedSession => "dismissed_session",
            Effect::Nothing => "nothing",
            Effect::Failed => "failed",
        }
    }
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

    /// The tool this harness gives the model for asking the user to pick
    /// from options, if it has one. The mapping instructions name it;
    /// naming another harness's tool sends the model after something it
    /// cannot call.
    fn ask_tool(&self) -> Option<&'static str> {
        None
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
/// Is this one of converge's own MCP tools? The server is registered as
/// `converge` everywhere, so its name is in the tool's, whatever the
/// harness composes ids from (`mcp__converge__x`, `converge_x`, …). A
/// harness that filters in its own config sends no name, which passes:
/// it already decided.
pub(crate) fn ours(tool: Option<&str>) -> bool {
    tool.is_none_or(|tool| tool.contains("converge"))
}

pub(crate) fn cwd_or_current(field: Option<&str>) -> PathBuf {
    field
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_converges_own_tools_are_ours() {
        // A harness that filters in its own config sends no name.
        assert!(ours(None));
        // The spellings the four harnesses actually produce.
        for tool in [
            "mcp__converge__project_bind",
            "converge_project_match",
            "converge.decision_add",
        ] {
            assert!(ours(Some(tool)), "{tool}");
        }
        // Everything else is someone else's call to make.
        for tool in ["Shell", "Read", "mcp__github__create_issue", "bash"] {
            assert!(!ours(Some(tool)), "{tool}");
        }
    }
}
