//! The hook entrypoints — the harness invokes these, directly (a hook
//! command) or through a shim (opencode's plugin); they never prompt and
//! they never fail the session (best-effort output, always exit 0 from
//! `run`).
//!
//! Everything tool-specific — which fields arrive, what the answer must
//! look like, where the transcript lives — belongs to [`crate::harness`].
//! What is left here is the part that is the same everywhere.
//!
//! Ported from the validated POC (`poc-mapping`): the inject rules
//! wording is contract-like — agents act on it — so it changes carefully.
//!
//! - `inject` (session start): read the three-state marker, emit the
//!   context block — bound (binding + decision index), disabled
//!   (stay-quiet rules), unbound (the mapping rules), or unreadable.
//! - `ctx` (pre-tool, matched on converge tools): merge `cwd` + git
//!   remote into the tool input, so the server ranks candidates without
//!   the LLM gathering anything.
//! - `mark` (post-tool, matched on the binding tools): perform the
//!   **local effect** — parse the tool response and write the marker at
//!   the git root. The LLM only ever chose; the write is deterministic.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use converge_client::{DecisionFilter, Pagination, ProjectId, SignalFilter, SignalStatus};
use serde_json::{Value, json};

use crate::config::Config;
use crate::harness::{Effect, Harness, Kind, Response};
use crate::marker::{self, State};

/// Whatever the harness put on stdin, before it means anything.
fn raw() -> Value {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        return Value::Null;
    }
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// Answer in the harness's own dialect — or stay quiet, which is not
/// the same as answering `null`.
fn respond(harness: &dyn Harness, response: Response) {
    if let Some(value) = harness.emit(response) {
        println!("{value}");
    }
}

// ─── session start ───────────────────────────────────────────────────────────

pub async fn inject(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());

    // Only a bound project's index is worth carrying in every request;
    // the other states are instructions, said once.
    let mut sticky = false;
    let mut degraded = false;
    let mut state = "unbound";
    let (context, system) = match marker::find(&payload.cwd) {
        // The visible line comes from bound() too: it must reflect what
        // the fetch actually found, not just that a marker exists.
        Ok(State::Bound { project, .. }) => {
            let (context, system, fallback) = bound(project).await;
            sticky = true;
            degraded = fallback;
            state = "bound";
            (context, system)
        }
        Ok(State::Disabled { .. }) => {
            // A standing rule, and short: worth carrying in every request.
            sticky = true;
            state = "disabled";
            (
                "## Converge — disabled for this repo\n\
             Converge is off here (`.converge` has `disable = true`). Do NOT \
             suggest mapping. To re-enable, remove that line from `.converge` \
             (or delete the file)."
                    .to_string(),
                "Converge: disabled for this repo".to_string(),
            )
        }
        Ok(State::Unbound) => (
            unbound(harness.ask_tool()),
            // The deterministic path leads: the human can always bind via
            // the TTY picker even when the model skips the injected flow.
            "Converge: repo unmapped — run `converge project init` to bind \
             (or let the agent suggest candidates)"
                .to_string(),
        ),
        Err(_) => {
            state = "unreadable";
            (
                "## Converge — marker unreadable\n\
             `.converge` exists but has neither `project_id` nor `disable`. \
             Re-link: call `project_match`, then `project_bind` — or tell \
             the user to run `converge project init --rebind`."
                    .to_string(),
                "Converge: marker unreadable — re-link".to_string(),
            )
        }
    };

    let system = match version_notice() {
        Some(notice) => format!("{system} · {notice}"),
        None => system,
    };
    respond(
        harness,
        Response::Inject {
            context,
            system,
            sticky,
            degraded,
            state,
        },
    );
    Ok(())
}

/// The unbound rules — the POC's tested wording, tool names updated to
/// the `resource_operation` palette. Step 3 names the harness's own
/// ask-the-user tool; the Claude Code text is the validated original,
/// the others say the same thing without a tool name they cannot call.
fn unbound(ask: Option<&str>) -> String {
    let (present, manual) = match ask {
        Some("AskUserQuestion") => (
            "present them via `AskUserQuestion`".to_string(),
            "tell the user the built-in 'Type something' is MANUAL MAPPING \
             — an existing id links, a new name creates"
                .to_string(),
        ),
        Some(tool) => (
            format!(
                "present them to the user as a numbered choice and wait for the \
                 answer (use your `{tool}` tool if you have it)"
            ),
            "also offer MANUAL MAPPING — an existing id links, a new name \
             creates"
                .to_string(),
        ),
        None => (
            "present them to the user as a numbered choice and wait for the \
             answer (use your ask-the-user tool if you have one)"
                .to_string(),
            "also offer MANUAL MAPPING — an existing id links, a new name \
             creates"
                .to_string(),
        ),
    };
    format!(
        "## Converge — this repo is UNMAPPED\n\
There is no `.converge` marker, so project memory is unavailable until \
this working tree is linked to a converge project.\n\n\
**Do this now, proactively, without being asked:**\n\
1. Call `project_match` (no arguments — a hook fills in the context).\n\
2. If the response carries an outcome (`project_id` or `disable: \
true`), the server already asked the user and a hook writes the marker \
— you are done; do NOT render your own menu.\n\
3. If it carries `candidates`: {present}. \
Label every candidate `name (group)` — the same project name can \
exist in several groups, and the group is what decides who sees the \
memory. One option per candidate + a 'Disable Converge for this repo' \
option; {manual}. Then `project_bind` with \
the pick, or `project_dismiss` scope='repo' to disable.\n\
4. When the user chooses to CREATE a project, you must also settle \
WHERE: ask which group from the response's `groups` (label `name \
(kind)`) — or offer a new group via `group_add` (ask shared vs \
personal). Placing a project decides who can see it; never assume a \
group, even when only one exists.\n\n\
Do NOT write `.converge` yourself — the hooks do it. Start with step 1 \
right away."
    )
}

/// The bound block: binding + a compact decision index + unjudged
/// signals, fetched best-effort (a hook must not fail the session
/// because the server is down — the binding itself is local knowledge).
/// What the bound block fetches: the project name, the decision index
/// lines, and the proposed-signal lines. `None` = the server answered
/// but doesn't know the project for this account.
type Index = Option<(String, Vec<String>, Vec<String>)>;

/// Index-fetch budget. Past it, degrade to the cached index (marked
/// stale) or the unavailable line — a session start must never hang on
/// a wedged server.
const BUDGET: std::time::Duration = std::time::Duration::from_secs(4);

/// Hook-triggered self-update: spawn `converge update` fully detached
/// and return immediately — a hook must never wait on a download. At
/// most one attempt per day (stamp file), only when the skew check says
/// the server is ahead, and `[update] auto = false` opts out. The swap
/// is safe under running sessions (old inode keeps serving them); the
/// next session notices the version change and says so.
fn maybe_self_update(auto: bool) {
    if !auto {
        return;
    }
    // One attempt per day, claimed atomically: hooks run concurrently
    // (subagents, opencode's parallel requests), and two `converge
    // update`s racing on the same staging path is how a machine ends up
    // with no binary. `create_new` on a dated file lets exactly one
    // process through; yesterday's file is swept on the way.
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / (24 * 3600))
        .unwrap_or(0);
    let Some(stamp) = cache_file(&format!("update-attempt-{day}")) else {
        return;
    };
    let _ = stamp.parent().map(std::fs::create_dir_all);
    let claimed = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stamp)
        .is_ok();
    if !claimed {
        return;
    }
    if let Some(old) = cache_file(&format!("update-attempt-{}", day.saturating_sub(1))) {
        let _ = std::fs::remove_file(old);
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::process::Stdio;
    let mut update = Command::new(exe);
    update
        .arg("update")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group: the update outlives the hook and the session.
        update.process_group(0);
    }
    let _ = update.spawn();
}

/// One-time notice after a version change (auto or manual update): the
/// previous inject's binary version is stamped; a mismatch means the
/// binary changed underneath the user since last session start.
fn version_notice() -> Option<String> {
    let current = env!("CARGO_PKG_VERSION");
    let stamp = cache_file("last-version")?;
    let previous = std::fs::read_to_string(&stamp).unwrap_or_default();
    let _ = stamp.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(&stamp, current);
    (!previous.is_empty() && previous != current)
        .then(|| format!("self-updated v{previous} → v{current}"))
}

fn cache_file(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".converge/cache").join(name))
}

/// Last-good index cache: the successful block, one file per project.
/// Served only on fetch failure, and always labelled with its age —
/// a stale index must read as stale, never as fresh.
fn cache_path(project: ProjectId) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".converge/cache")
            .join(format!("index-{project}.md")),
    )
}

/// Returns `(context block, visible summary line)` — the summary states
/// what was actually injected (or why nothing was), never a bare ✓.
/// Bound for the skew check: it runs after the index budget is spent,
/// and a wedged server must not turn a session start into a hang.
const SKEW_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// Also answers whether the block is a fallback (the third element): a
/// cached or unavailable index, or a project the server disowns.
async fn bound(project: ProjectId) -> (String, String, bool) {
    // Loaded once: `Config::load` may run a `token_cmd` (a password
    // manager call), and it used to run twice per session start.
    let loaded = Config::load();
    let auto_update = loaded.as_ref().map(|c| c.auto_update).unwrap_or(true);
    let client = loaded.and_then(|config| config.client());
    let fetch = async {
        let client = match &client {
            Ok(client) => client.clone(),
            Err(e) => return Err(anyhow::anyhow!("{e}")),
        };
        // ACL: an invisible project answers exactly like a missing one —
        // don't dress that up as an empty decision index.
        let Some(found) = client.project_get(project).await? else {
            return Ok(None);
        };
        let name = found.name;
        let decisions = client
            .decision_list(
                &DecisionFilter {
                    project: Some(project),
                    ..Default::default()
                },
                &Pagination {
                    limit: Some(30),
                    cursor: None,
                },
            )
            .await?
            .items
            .iter()
            .map(|d| {
                format!(
                    "- {} [{}]",
                    d.title,
                    format!("{:?}", d.status).to_lowercase()
                )
            })
            .collect();
        let signals = client
            .signal_list(
                &SignalFilter {
                    project: Some(project),
                    status: Some(SignalStatus::Proposed),
                    ..Default::default()
                },
                &Pagination {
                    limit: Some(10),
                    cursor: None,
                },
            )
            .await?
            .items
            .iter()
            .map(|s| {
                format!(
                    "- [{}/{}] {} ({})",
                    format!("{:?}", s.tier).to_lowercase(),
                    s.kind,
                    s.title,
                    s.id
                )
            })
            .collect();
        Ok(Some((name, decisions, signals)))
    };
    // Budget timeout counts as a fetch failure — same degradation path.
    let fetched: Result<Index> = match tokio::time::timeout(BUDGET, fetch).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("index fetch exceeded {BUDGET:?}")),
    };

    // The daily-cached skew nudge rides the bound path (the one that
    // already talks to the server); its failure is silence, not noise.
    // Skipped when the index fetch failed — a wedged server would hang
    // this call too, and the budget already spent its patience.
    let fetch_ok = fetched.is_ok();
    let (block, system, degraded) = match fetched {
        Ok(None) => {
            // The server disowned the project; a lingering cached index
            // would resurrect it on the next outage.
            if let Some(path) = cache_path(project) {
                let _ = std::fs::remove_file(path);
            }
            (
                format!(
                    "## Converge memory — project {project}\n\
                 This working tree is bound to converge project `{project}`, \
                 but the server doesn't know it for this account — the project \
                 was deleted, or this token's user isn't a member of its \
                 group. Ask a group owner to add you, or re-bind with \
                 `project_match`."
                ),
                format!(
                    "Converge: bound to {project}, but the server doesn't \
                     know it — re-bind (`converge project init --rebind`)"
                ),
                // Authoritative, not a fallback: retrying will not change it.
                false,
            )
        }
        Ok(Some((name, decisions, signals))) => {
            // The list limits cap what we can count; say "N+" at the cap
            // instead of understating a bigger corpus as exactly N.
            let counted = |n: usize, cap: usize| {
                if n >= cap {
                    format!("{n}+")
                } else {
                    n.to_string()
                }
            };
            let system = format!(
                "Converge: \"{name}\" — {} decision(s), {} open signal(s) ✓",
                counted(decisions.len(), 30),
                counted(signals.len(), 10),
            );
            let mut block = if decisions.is_empty() {
                format!(
                    "## Converge memory — project \"{name}\" ({project})\n\
                     This working tree is bound to converge project `{project}`; \
                     project memory is active. No decisions are recorded yet — use \
                     `decision_add` when a design decision lands, and record the \
                     conversation (`session_ensure` + `message_add`) so decisions \
                     can cite their evidence."
                )
            } else {
                format!(
                    "## Converge memory — project \"{name}\" ({project})\n\
                     This working tree is bound to converge project `{project}`; \
                     project memory is active. Decisions below are in force — \
                     `decision_get` for the full record before re-deciding a \
                     settled topic; `decision_add` (with `supersedes`/`evidence`) \
                     when a new decision lands.\n\nDecisions:\n{}",
                    decisions.join("\n")
                )
            };
            if !signals.is_empty() {
                block.push_str(&format!(
                    "\n\nProposed signals (unjudged observations touching this \
                     project — raise conflict-tier ones with the user \
                     proactively; `signal_list` for the full record, then \
                     `signal_resolve` with THEIR verdict, never your own):\n{}",
                    signals.join("\n")
                ));
            }
            // Last-good cache: written on success, served on failure.
            // A ghost binding (Ok(None)) clears it instead — the server
            // authoritatively disowned the project.
            if let Some(path) = cache_path(project) {
                let _ = path.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(&path, &block);
            }
            (block, system, false)
        }
        Err(_) => match cache_path(project).filter(|p| p.exists()) {
            Some(path) => {
                let age = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| format!("{}h", d.as_secs() / 3600))
                    .unwrap_or_else(|| "unknown".into());
                let cached = std::fs::read_to_string(&path).unwrap_or_default();
                // The age stays out of the block: a harness that carries the
                // block in every request would bust its prompt cache on a
                // clock tick. It is in the visible line instead.
                (
                    format!(
                        "{cached}\n\n**Stale index**: the server was unreachable \
                         at session start; the index above is a local cache. \
                         It may miss recent records — treat it as a map, and \
                         `decision_list` to confirm when it matters."
                    ),
                    format!("Converge: server unreachable — cached index ({age} old)"),
                    true,
                )
            }
            None => (
                format!(
                    "## Converge memory — project {project}\n\
                     This working tree is bound to converge project `{project}`, \
                     but the server was unreachable when this session started — \
                     the decision index is unavailable. Tools may still work; \
                     `decision_list` fetches the index on demand."
                ),
                format!("Converge: bound to {project} (server unreachable — index unavailable)"),
                true,
            ),
        },
    };
    // The skew check runs after the index and its cache are settled: a
    // harness that kills a slow hook must not lose a fetched index to
    // the version nudge that follows it.
    let skew = match &client {
        Ok(client) if fetch_ok => {
            tokio::time::timeout(SKEW_BUDGET, crate::skew::check_cached(client))
                .await
                .ok()
                .flatten()
        }
        _ => None,
    };
    // Skew present = the server is ahead of this binary: the one moment
    // auto-update has something to do. Fire-and-forget, at most daily.
    if skew.is_some() {
        maybe_self_update(auto_update);
    }
    // The nudge is for the operator, not the model: it says to run
    // commands, and it goes in the visible line only.
    let system = match skew {
        Some(warning) => format!("{system} · {warning}"),
        None => system,
    };
    (block, system, degraded)
}

// ─── session end: transcript → evidence ──────────────────────────────────────

pub async fn sync(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    // Best effort throughout: a sync problem must never surface as a
    // session failure. The quiet paths just return.
    if let Err(e) = try_sync(harness).await {
        respond(
            harness,
            Response::Notice {
                system: format!("Converge: sync skipped — {e}"),
            },
        );
    }
    Ok(())
}

async fn try_sync(harness: &dyn Harness) -> Result<()> {
    let payload = harness.parse(&raw());
    // A harness that records nothing readable has no evidence to push.
    let Some(transcript) = payload.transcript.as_ref() else {
        return Ok(());
    };

    // Only bound repos sync; unbound and disabled stay quiet.
    let State::Bound { project, .. } = marker::find(&payload.cwd)? else {
        return Ok(());
    };

    let parsed = harness.transcript(transcript)?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok(()); // no session id in the content — nothing to key on
    };

    let mut marks = crate::watermark::Watermarks::load()?;
    let key = transcript.key();
    // What has not been pushed yet — by message id where the harness has
    // them (a transcript edited in place still syncs right), by count
    // otherwise, where a shrunk transcript sends nothing rather than
    // duplicates.
    let fresh = marks.pending(&key, &parsed.turns);
    if fresh.is_empty() {
        marks.done(&key, &parsed.turns);
        marks.save()?;
        return Ok(());
    }

    let config = Config::load()?;
    let client = config.client()?;

    let session = client
        .session_ensure(&converge_client::NewSession {
            project_id: project,
            kind: converge_client::SessionKind::Transcript,
            external,
            title: crate::transcript::title(&parsed.turns, &format!("{} session", harness.label())),
        })
        .await?;
    let messages: Vec<_> = fresh
        .iter()
        .map(|t| converge_client::NewMessage {
            speaker: t.speaker.clone(),
            body: t.body.clone(),
            sent_at: t.sent_at,
        })
        .collect();
    let added = messages.len();
    client.message_add(session, &messages).await?;

    marks.done(&key, &parsed.turns);
    marks.save()?;
    respond(
        harness,
        Response::Notice {
            system: format!("Converge: synced {added} message(s) to evidence ✓"),
        },
    );
    Ok(())
}

// ─── pre-tool: context collector ──────────────────────────────────────────

pub fn ctx(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());
    // Someone else's tool call: say nothing rather than graft `cwd` onto
    // arguments that never asked for it.
    if !crate::harness::ours(payload.tool_name.as_deref()) {
        respond(harness, Response::Silent);
        return Ok(());
    }

    let mut merged = payload.tool_input;
    if !merged.is_object() {
        merged = json!({});
    }
    merged["cwd"] = json!(payload.cwd.to_string_lossy());
    if let Some(remote) = remote(&payload.cwd) {
        merged["remote"] = json!(remote);
    }

    respond(harness, Response::Ctx { tool_input: merged });
    Ok(())
}

fn remote(cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!url.is_empty()).then_some(url)
}

// ─── post-tool: the local effect ──────────────────────────────────────────

pub fn mark(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());
    if !crate::harness::ours(payload.tool_name.as_deref()) {
        respond(harness, Response::Silent);
        return Ok(());
    }
    let root = marker::root(&payload.cwd);

    let response = tool_json(&payload.tool_response);
    let (effect, system) = match effect(&response, &root) {
        Outcome::Bound(name) => (
            Effect::Bound,
            Some(format!(
                "Converge: linked this repo to \"{name}\" (wrote .converge) ✓"
            )),
        ),
        Outcome::Disabled => (
            Effect::Disabled,
            Some("Converge: disabled for this repo (wrote .converge)".to_string()),
        ),
        // Nothing on disk, but a harness with state of its own must know.
        Outcome::DismissedSession => (Effect::DismissedSession, None),
        // A skip or an unrecognized payload: no local effect, no noise.
        Outcome::Nothing => (Effect::Nothing, None),
        Outcome::Failed(err) => (
            Effect::Failed,
            Some(format!("Converge: could not write .converge — {err}")),
        ),
    };
    respond(harness, Response::Marked { effect, system });
    Ok(())
}

enum Outcome {
    Bound(String),
    Disabled,
    DismissedSession,
    Nothing,
    Failed(String),
}

/// Interpret a binding tool's response: `{project_id, name}` → bound,
/// `{disable: true}` → disabled, `{dismissed: "session"}` → nothing on
/// disk but a real answer, anything else → no local effect.
fn effect(response: &Value, root: &Path) -> Outcome {
    if response["disable"].as_bool() == Some(true) {
        return match marker::write_disabled(root) {
            Ok(_) => Outcome::Disabled,
            Err(e) => Outcome::Failed(e.to_string()),
        };
    }
    if let Some(id) = response["project_id"].as_str() {
        let Ok(project) = id.parse() else {
            return Outcome::Failed(format!("`{id}` is not a project id"));
        };
        let name = response["name"].as_str().unwrap_or(id).to_string();
        return match marker::write_bound(root, project, &name) {
            Ok(_) => Outcome::Bound(name),
            Err(e) => Outcome::Failed(e.to_string()),
        };
    }
    if response["dismissed"].as_str() == Some("session") {
        return Outcome::DismissedSession;
    }
    Outcome::Nothing
}

/// MCP tool responses arrive as a content array (`[{type: "text", text:
/// "<json>"}]`), that array wrapped in a result envelope (`{content:
/// [...], isError}` — what Claude Code hands PostToolUse today), a plain
/// object, or a string — accept all four (the POC's leniency, extended).
fn tool_json(response: &Value) -> Value {
    let text = match response {
        Value::Array(items) => items.first().and_then(|i| i["text"].as_str()),
        Value::Object(_) if response["content"].is_array() => {
            response["content"][0]["text"].as_str()
        }
        Value::Object(_) => {
            if response["text"].is_string() {
                response["text"].as_str()
            } else {
                return response.clone();
            }
        }
        Value::String(s) => Some(s.as_str()),
        _ => None,
    };
    text.and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cvg-hook-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn effects_from_tool_payload_shapes() {
        let root = temp();
        let id = ProjectId::new();

        // MCP content-array shape → bound.
        let response = tool_json(&json!([
            { "type": "text", "text": format!("{{\"project_id\":\"{id}\",\"name\":\"gw\"}}") }
        ]));
        assert!(matches!(effect(&response, &root), Outcome::Bound(n) if n == "gw"));
        assert!(matches!(
            marker::find(&root).unwrap(),
            State::Bound { project, .. } if project == id
        ));

        // The result envelope Claude Code hands PostToolUse → same.
        let response = tool_json(&json!({
            "content": [
                { "type": "text", "text": format!("{{\"project_id\":\"{id}\",\"name\":\"gw\"}}") }
            ],
            "isError": false,
        }));
        assert!(matches!(effect(&response, &root), Outcome::Bound(n) if n == "gw"));

        // Dismiss repo → disabled (overwrites).
        let response = tool_json(&json!({ "dismissed": "repo", "disable": true }));
        assert!(matches!(effect(&response, &root), Outcome::Disabled));
        assert!(matches!(
            marker::find(&root).unwrap(),
            State::Disabled { .. }
        ));

        // A session-scope dismiss is reported as such; skips are nothing.
        assert!(matches!(
            effect(&json!({ "dismissed": "session", "disable": false }), &root),
            Outcome::DismissedSession
        ));
        for benign in [
            json!({ "skipped": true }),
            json!({ "elicitation": false }),
            Value::Null,
        ] {
            assert!(matches!(effect(&benign, &root), Outcome::Nothing));
        }

        // Garbage id fails loudly, writes nothing over the disabled state.
        let response = json!({ "project_id": "nonsense" });
        assert!(matches!(effect(&response, &root), Outcome::Failed(_)));
        assert!(matches!(
            marker::find(&root).unwrap(),
            State::Disabled { .. }
        ));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
