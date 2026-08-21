# converge-cli

There are three parts.

## Part 1 — the core (harness-agnostic)

### Credentials and server

This step handles authentication: it works out the server address and
gets a credential. There are two ways to get one:
- **browser pairing** — the person signs in through a browser, and the
  grant that comes back becomes the stored credential. Used when the
  server offers it;
- **a ready token** — the person creates it in the web UI or on the
  server host and pastes it when asked. Always works.

Either way, the credential is checked with one request to the server
*before* anything is written to disk: if the check fails, nothing is
saved. After that the config file is read on every run of every command.
Environment variables override it, and the secret can come from an
external command instead of sitting in the file in plain text.

### Self-distribution

The binary updates itself from a signed release. The previous version
stays on disk next to it, and one command brings it back. There is no
automatic rollback, because every check happens before the swap: if the
signature does not match, if the checksum does not match, or if the new
binary does not run, the update simply does not happen and the working
binary is untouched. A version difference with the server is reported,
never enforced.

### Binding state

A file named `.converge` in the repository root holds the id of a
project on the server. There are three states, and none of them is a
silent default:
- **bound** — the file holds a project id, and sessions resolve to it;
- **disabled** — the file says converge is not wanted here, and the
  integration stays quiet;
- **not bound** — there is no file, so a binding should be offered.

The file is found by walking up from the working directory; the nearest
one wins. A new one is written in the git repository root, so a session
started in a subdirectory of a monorepo resolves the same way. A file
with neither an id nor the disable flag is an error we say out loud, not
something to guess about.

Resolution is always by id, never by name: names change, ids do not. And
the file is committed on purpose: a repository bound once is bound for
the whole team, on every machine and every harness.

### Transcript

The conversation with the agent is written to a transcript, and converge
copies it to the server, so that decisions can point at the messages
they came from.

Only visible prose is kept: what the person and the agent said. Tool
calls, tool results and reasoning are dropped — for a link back to the
conversation they are noise.

A session is identified by an id taken from the transcript content, not
from the file name. Sending the same conversation again adds to the
existing record instead of creating a second one. Only what appeared
since the last sync is sent: the machine remembers how many messages of
each transcript it has already sent. It counts messages, not bytes, so
the title taken from the person's first line stays the same from sync to
sync.

### Manual control of project binding

A binding can also be set without an agent, by running
`converge project init`. The command shows the projects that exist on
the server, lets you create a new one, and writes the choice into
`.converge`. Besides binding it can do two more things: disable the
repository, and replace an existing binding. The last one is needed when
the marker points at a project the server does not know.

This is the manual path, the repair path, and the only way to bind on a
harness that automates nothing. Disabling works offline: saying no does
not need a server. Nothing is bound silently here either — the list is
shown, and the person picks.

## Part 2 — setup (harness-specific)

This part is tied to one specific harness: find it, work out where its
settings live, write calls to converge-cli into them, and register the
MCP server. It runs once per machine as `converge init` and is
idempotent, so running it again after an update or after the binary has
moved is how you repair things.

### Harness contracts

A harness integration has two independent object-safe contracts. `Setup`
composes the machine-setup capabilities; `Adapter` belongs only to hook
runtime. They are selected through separate registries and never receive
one another. The only value setup passes to a hook is `HarnessId`, written
into the command line.

```rust
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

enum HarnessId {
    ClaudeCode,
    Codex,
    Cursor,
}

// Part 2: small setup capabilities.

trait Executable {
    fn exists(&self) -> bool;
    fn settings_path(&self) -> Result<PathBuf>;

    fn install_hooks(
        &self,
        converge_executable: &Path,
    ) -> Result<Vec<HookInstall>>;

    fn verify_hooks(&self) -> Result<Vec<HookCheck>>;
}

trait McpRegistrar {
    fn register(
        &self,
        endpoint: &McpEndpoint,
    ) -> Result<InstallOutcome>;

    fn unregister(&self) -> Result<InstallOutcome>;
    fn verify(&self, endpoint: &McpEndpoint) -> Result<Verification>;
}

// The complete part-2 contract. `converge init` works with `&dyn Setup`.
trait Setup: Executable + McpRegistrar {
    fn id(&self) -> HarnessId;
}

// Part 3: envelope translation only. It is intentionally not a
// supertrait of Setup and cannot reach installation through this API.

trait Adapter {
    fn parse(
        &self,
        event: HookEvent,
        request: Value,
    ) -> Result<Option<HookRequest>>;

    fn render(
        &self,
        event: HookEvent,
        response: HookResponse,
    ) -> Result<Option<Value>>;
}

// Separate runtime registries preserve the boundary between the parts.

fn setup(id: HarnessId) -> &'static dyn Setup {
    todo!("setup registry")
}

fn installed() -> Vec<&'static dyn Setup> {
    todo!("detect every known setup implementation")
}

fn adapter(id: HarnessId) -> &'static dyn Adapter {
    todo!("adapter registry")
}

// The command is the complete seam between setup and hook runtime.

fn hook_command(exe: &Path, event: HookEvent, id: HarnessId) -> String {
    todo!("converge hook <event> --harness <id>")
}

fn hook_probe(event: HookEvent) -> String {
    todo!("recognise old and current commands by their job")
}

#[derive(Clone, Copy)]
enum HookEvent {
    Inject,
    Ctx,
    Mark,
    Sync,
}

enum HookRequest {
    Inject {
        cwd: PathBuf,
    },
    Ctx {
        cwd: PathBuf,
        tool_input: Value,
    },
    Mark {
        cwd: PathBuf,
        tool_response: Value,
    },
    Sync {
        cwd: PathBuf,
        session: String,
        source: String,
        turns: Vec<Turn>,
    },
}

enum HookResponse {
    Context {
        additional_context: String,
        visible_status: Option<String>,
    },
    UpdatedToolInput(Value),
    Status(String),
    Silent,
}

struct HookInstall {
    event: HookEvent,
    outcome: InstallOutcome,
}

struct HookCheck {
    event: HookEvent,
    state: Verification,
}

enum InstallOutcome {
    Changed,
    Unchanged,
    Unsupported { manual_instructions: String },
}

enum Verification {
    Working,
    Missing,
    Different,
    NeedsApproval,
    Unsupported { manual_instructions: String },
}

struct McpEndpoint {
    server: String,
    credential: Credential,
}

enum Credential {
    Bearer(String),
    EnvVar(String),
}

// A harness adapter normalises its transcript into this shared type.
struct Turn;
```

### Find the harness

Whether it is installed, and where its settings are. Every harness keeps
them in its own place and in its own format.

### Install the hooks

Register hooks that call converge-cli on the harness's lifecycle events,
without touching anything the user put in those settings themselves.

How exactly is up to the implementation for that harness: one has JSON
to edit, another TOML, and one that can only load plugins gets a
generated file with code that calls the binary itself. The result is the
same: after this step the harness calls converge-cli. Where we write
that file, it also converts the input into our own format, so part 3
knows nothing about that harness.

### Register the MCP server

Either a CLI call or a config edit, depending on the harness. The
registration carries the server URL and a credential, so reconfiguring
replaces it rather than leaving it as it was.

### Verify

Written is not the same as working: a harness may keep hooks behind user
approval, or turn some events off in certain modes. Setup reports what
is connected and what is not.

### Talking to the user

Part 2 is the only place where converge talks to a person directly.
Everything else happens inside agent sessions, so this is where we both
ask and report.

We ask for very little: the server address, a browser sign-in or a ready
token, and permission to register the MCP server. The rest is worked out
on its own.

Setup reports on each step separately: what was set up, what was already
set up before, what was skipped, and how to finish it by hand later.
This is not decoration. Hooks and registrations are not visible to the
eye, and without a report a person cannot tell "it works" from "it
quietly did not install".

Project matching stays stateless. If `project_match` returns candidates,
the calling agent asks the user through the harness's ordinary
interaction and then calls `project_bind` or `project_dismiss`. Setup does
not detect or persist a question capability, and the injected
instructions do not name a harness-specific question tool.

### Adapters

The commands in part 3 read JSON and write JSON, but each harness has
its own. Claude Code puts the working directory right into the request;
Cursor does not include it at all and passes it in an environment
variable instead. The reply that carries session context sits in a
nested object for one harness and in a top-level field for another, and
the line shown to the user exists in one and has nowhere to go in
another.

An adapter is a pair of functions per harness: parse the incoming
envelope, build the outgoing one. The shared core runs between them, and
what it returns is identical down to the last character. For a harness
where we generate the plugin ourselves the adapter almost disappears: we
define the envelope, the plugin already speaks it, and there is nothing
to parse or build.

Which adapter to use is read from the command line itself. Setup writes
that line in full, including the harness id —
`converge hook inject --harness <id>`. This is the only thing passed
from part 2 to part 3: no shared object, no state file, no call from one
part into the other. The same way `.converge` connects whoever writes
the marker with whoever reads it.

## Part 3 — serving the hooks (harness-agnostic, except the format)

Four commands, called by the harness and never by a person. What each
one does is the same everywhere; only the envelope differs.

### Session start — `hook inject`

Read the marker and return what the session should begin with: silence
if the repository is disabled, binding rules if it is not bound, and the
**project index** if it is bound — the decisions in force and the
signals nobody has judged yet, fetched from the server and handed over
as context. This is a push of something the agent could pull on its own:
the same records are available through the MCP tools, but only if the
model thinks to ask, and it cannot think to ask about a project nobody
told it about. The request has a time limit and degrades: first to a
local copy labelled with its age, then to an honest "not available". A
stale index has to read as stale.

### Session end — `hook sync`

Send the messages that appeared since last time to the server. Bound
repositories only.

### Pre-tool — `hook ctx`

On converge tool calls, add the working directory and the repository
remote. The model does not spend a turn working out where it is, and
cannot get it wrong.

### Post-tool — `hook mark`

On the binding tools, do the local thing the answer implies: write the
marker, or deliberately nothing. The model only made a choice; the
writing is plain deterministic code.

### Invariants

These commands never ask anything. They never break the session — any
failure degrades into a quieter answer. They can run repeatedly, because
the event that triggers them may fire more than once. Anything that goes
to the network has a time limit: a session start must never hang on a
stuck server.
