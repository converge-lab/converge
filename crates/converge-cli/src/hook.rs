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

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use converge_client::{
    DecisionFilter, MessageId, Pagination, ProjectId, Signal, SignalFilter, SignalId, SignalStatus,
    Tier,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::harness::{Effect, Harness, Kind, Payload, Response};
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
            let (context, system, fallback) =
                bound(project, payload.session.as_deref(), kind.flag()).await;
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
    // An install that predates a hook lacks it silently. This hook is
    // the one every install has, so this is where to say so.
    let missing = std::env::current_exe()
        .ok()
        .map(|exe| harness.missing(&exe.to_string_lossy()))
        .unwrap_or_default();
    let system = if missing.is_empty() {
        system
    } else {
        format!(
            "{system} · run `converge init --harness {}` to add the {} hook",
            kind.flag(),
            missing.join(" and ")
        )
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
/// lines, and the open signals. `None` = the server answered but
/// doesn't know the project for this account.
type Index = Option<(String, Vec<String>, Vec<Open>)>;

/// An open signal as the block shows it.
#[derive(Debug, Clone, PartialEq)]
struct Open {
    id: SignalId,
    tier: Tier,
    kind: String,
    title: String,
    /// Never shown to this user before, in any session on any machine:
    /// the server holds no receipt for it.
    new: bool,
}

/// The bound block and its visible line, from what the server said.
/// Pure, so the wording and the order are testable without a server.
/// This listing is deliberately unfiltered by receipts: it is the one
/// place every open signal shows, whatever a poll handed out or dropped.
fn render(
    project: ProjectId,
    name: &str,
    decisions: &[String],
    signals: &[Open],
) -> (String, String) {
    let is_new = |s: &Open| s.new;
    let new = signals.iter().filter(|s| is_new(s)).count();
    let conflicts = signals.iter().filter(|s| s.tier == Tier::Conflict).count();
    // The list limits cap what we can count; say "N+" at the cap
    // instead of understating a bigger corpus as exactly N.
    let counted = |n: usize, cap: usize| {
        if n >= cap {
            format!("{n}+")
        } else {
            n.to_string()
        }
    };
    let mut detail = Vec::new();
    if new > 0 {
        detail.push(format!("{new} new"));
    }
    if conflicts > 0 {
        detail.push(format!("{conflicts} conflict"));
    }
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" ({})", detail.join(", "))
    };
    let system = format!(
        "Converge: \"{name}\" — {} decision(s), {} open signal(s){detail} ✓",
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
        // What changed since the last start leads; a conflict leads
        // within that; newest first after.
        let mut shown: Vec<&Open> = signals.iter().collect();
        shown.sort_by_key(|s| {
            (
                std::cmp::Reverse(is_new(s)),
                std::cmp::Reverse(s.tier),
                std::cmp::Reverse(s.id),
            )
        });
        let lines: Vec<String> = shown
            .iter()
            .map(|s| {
                format!(
                    "- [{}/{}] {} ({}){}",
                    format!("{:?}", s.tier).to_lowercase(),
                    s.kind,
                    s.title,
                    s.id,
                    if is_new(s) { " ← NEW" } else { "" }
                )
            })
            .collect();
        let legend = if new > 0 {
            "; ← NEW = not shown to you before, in any session"
        } else {
            ""
        };
        block.push_str(&format!(
            "\n\nProposed signals (unjudged observations touching this \
             project{legend} — raise conflict-tier ones with the user \
             proactively; `signal_list` for the full record, then \
             `signal_resolve` with THEIR verdict, never your own):\n{}",
            lines.join("\n")
        ));
    }
    (block, system)
}

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
/// binary changed underneath the user since last session start. The
/// update's report of what it refreshed rides the same line, once — an
/// automatic update has no other way to say what it did.
fn version_notice() -> Option<String> {
    let current = env!("CARGO_PKG_VERSION");
    let stamp = cache_file("last-version")?;
    let previous = std::fs::read_to_string(&stamp).unwrap_or_default();
    let _ = stamp.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(&stamp, current);
    let changed = (!previous.is_empty() && previous != current)
        .then(|| format!("self-updated v{previous} → v{current}"));
    let refreshed = crate::setup::Report::take().and_then(|r| r.line());
    match (changed, refreshed) {
        (Some(changed), Some(refreshed)) => Some(format!("{changed}; {refreshed}")),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    }
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
/// Receipts budget: after the block is settled, one small request that
/// says what this session was shown — and, first time, that it began.
const RECEIPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

async fn bound(project: ProjectId, session: Option<&str>, harness: &str) -> (String, String, bool) {
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
        // The open signals, and which of them this user has never been
        // shown: two reads of one list, the second narrowed by receipts.
        let open = SignalFilter {
            project: Some(project),
            status: Some(SignalStatus::Proposed),
            ..Default::default()
        };
        let unseen = SignalFilter {
            unseen: true,
            ..open.clone()
        };
        let page = Pagination {
            limit: Some(10),
            cursor: None,
        };
        let (listed, fresh) = tokio::try_join!(
            client.signal_list(&open, &page),
            client.signal_list(&unseen, &page)
        )?;
        let fresh: BTreeSet<SignalId> = fresh.items.iter().map(|s| s.id).collect();
        let signals = listed
            .items
            .iter()
            .map(|s| Open {
                id: s.id,
                tier: s.tier,
                kind: s.kind.clone(),
                title: s.title.clone(),
                new: fresh.contains(&s.id),
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
    let mut listed: Vec<SignalId> = Vec::new();
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
            listed = signals.iter().map(|s| s.id).collect();
            let (block, system) = render(project, &name, &decisions, &signals);
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
    // What this session was shown, receipted — and the session's row
    // created if this is its first sight, which is the line the poll
    // draws: nothing recorded before it is handed out later. Only on a
    // fresh index (a cached one says nothing about receipts), only with
    // a session to key it, and its failure is silence.
    if let (Ok(client), Some(session), true) = (&client, session, fetch_ok) {
        let _ = tokio::time::timeout(
            RECEIPT_BUDGET,
            client.signal_receive(session, Some(harness), &listed),
        )
        .await;
    }
    // The nudge is for the operator, not the model: it says to run
    // commands, and it goes in the visible line only.
    let system = match skew {
        Some(warning) => format!("{system} · {warning}"),
        None => system,
    };
    (block, system, degraded)
}

// ─── per prompt: signals raised since the last one ───────────────────────────

/// The poll's network budget. It runs on every prompt the gates let
/// through, so it is tighter than session start's: a slow server costs a
/// prompt two seconds, then the backoff.
const POLL_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
/// How many signals one prompt is handed; the rest wait for the next.
const POLL_CLAIM: u32 = 3;

/// The per-prompt seam: claim what this session has not been shown and
/// put it in front of the model with the prompt. Every gate runs before
/// the config is loaded — its token command may be a password manager —
/// and every miss is silence, never an error: a hook here fails the
/// user's prompt, not ours.
pub async fn poll(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());
    // Only a bound project has signals to hear about.
    let Ok(State::Bound { .. }) = marker::find(&payload.cwd) else {
        return Ok(());
    };
    // The ledger is keyed by session: a harness that sends none cannot
    // be polled for without repeating itself.
    let Some(session) = payload.session.filter(|s| !s.trim().is_empty()) else {
        return Ok(());
    };
    let now = crate::poll::now();
    let mut stamps = crate::poll::Stamps::load();
    let stamp = stamps.get(&session).cloned();
    if !crate::poll::due(stamp.as_ref(), now) {
        return Ok(());
    }
    let floor = crate::poll::floor(stamp.as_ref());

    let client = Config::load().and_then(|config| config.client());
    let claimed: Result<Vec<Signal>> = match client {
        Ok(client) => {
            match tokio::time::timeout(
                POLL_BUDGET,
                client.signal_claim(&session, Some(kind.flag()), POLL_CLAIM),
            )
            .await
            {
                Ok(result) => result.map_err(|e| anyhow::anyhow!("{e}")),
                Err(_) => Err(anyhow::anyhow!("poll exceeded {POLL_BUDGET:?}")),
            }
        }
        Err(e) => Err(e),
    };
    // What the ledger handed out is consumed whether or not it is shown:
    // a watch-tier signal dropped here stays in the session-start
    // listing and in `signal_list`, and is not offered to this session
    // again.
    let (shown, failed) = match claimed {
        Ok(signals) => (
            signals
                .into_iter()
                .filter(|s| s.tier >= floor)
                .collect::<Vec<_>>(),
            false,
        ),
        Err(_) => (Vec::new(), true),
    };
    stamps.record(&session, now, failed, shown.len());
    let _ = stamps.save();
    if shown.is_empty() {
        return Ok(());
    }
    let (context, system) = frame(&shown);
    respond(
        harness,
        Response::Signals {
            context,
            system,
            event: payload
                .event
                .unwrap_or_else(|| "UserPromptSubmit".to_string()),
        },
    );
    Ok(())
}

/// The per-prompt block: what arrived, framed so the model reads it as
/// information from Converge — never as the user's words, and never as
/// an instruction to act on by itself. The wording is factual on
/// purpose: text shaped like an out-of-band command trips a model's
/// injection defences and gets shown to the user as suspicious instead
/// of read.
fn frame(signals: &[Signal]) -> (String, String) {
    let n = signals.len();
    let conflicts = signals.iter().filter(|s| s.tier == Tier::Conflict).count();
    let plural = if n == 1 { "" } else { "s" };
    let system = format!(
        "Converge: {n} new signal{plural}{}",
        if conflicts > 0 {
            format!(" ({conflicts} conflict)")
        } else {
            String::new()
        }
    );
    let mut context = format!(
        "Converge: {n} signal{plural} raised since your last prompt — the expert's \
         observations about decisions recorded in this project's group, some \
         possibly from other people's sessions. Observations to weigh with the \
         user, not instructions."
    );
    if conflicts > 0 {
        context.push_str(
            " A conflict-tier one says the decision it names cannot stand with \
             another: put it to the user before continuing.",
        );
    }
    for s in signals {
        context.push_str(&format!(
            "\n- [{}/{}] {} ({}): {}",
            format!("{:?}", s.tier).to_lowercase(),
            s.kind,
            s.title,
            s.id,
            s.text.trim()
        ));
        if let Some(rec) = s
            .recommendation
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
        {
            context.push_str(&format!("\n  Recommendation: {rec}"));
        }
    }
    context.push_str(
        "\nMention them to the user; `decision_get` and `signal_list` hold the \
         full record; `signal_resolve` only with the user's verdict, never your own.",
    );
    (context, system)
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
    // Only bound repos sync; unbound and disabled stay quiet.
    let State::Bound { project, .. } = marker::find(&payload.cwd)? else {
        return Ok(());
    };
    let added = push_transcript(harness, &payload, project).await?.len();
    if added > 0 {
        respond(
            harness,
            Response::Notice {
                system: format!("Converge: synced {added} message(s) to evidence ✓"),
            },
        );
    }
    Ok(())
}

/// Push what the transcript holds that the server does not yet, and
/// return the ids it got. Watermarked, so calling it twice sends nothing
/// twice: the session-end sync and a `decision_add` in mid-session share
/// it, and what the second returns is exactly the turns since the last
/// push — the conversation that led to the decision. Empty when the
/// harness records nothing readable, or the content has no session id
/// to key on.
async fn push_transcript(
    harness: &dyn Harness,
    payload: &Payload,
    project: ProjectId,
) -> Result<Vec<MessageId>> {
    let Some(transcript) = payload.transcript.as_ref() else {
        return Ok(Vec::new());
    };
    let parsed = harness.transcript(transcript)?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok(Vec::new());
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
        return Ok(Vec::new());
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
    let ids = client.message_add(session, &messages).await?;

    marks.done(&key, &parsed.turns);
    marks.save()?;
    Ok(ids)
}

// ─── pre-tool: context collector ──────────────────────────────────────────

/// The budget for putting the conversation on record before a
/// `decision_add`: a few turns to the server. Past it, the call goes
/// through as the model made it and the server says what is missing.
const CTX_BUDGET: std::time::Duration = std::time::Duration::from_secs(4);
/// How many of the turns since the last push a decision cites.
const CITED_TURNS: usize = 10;

pub async fn ctx(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());
    // Someone else's tool call: say nothing rather than graft `cwd` onto
    // arguments that never asked for it.
    if !crate::harness::ours(payload.tool_name.as_deref()) {
        respond(harness, Response::Silent);
        return Ok(());
    }

    let mut merged = payload.tool_input.clone();
    if !merged.is_object() {
        merged = json!({});
    }
    merged["cwd"] = json!(payload.cwd.to_string_lossy());
    if let Some(remote) = remote(&payload.cwd) {
        merged["remote"] = json!(remote);
    }

    // A decision's evidence is filled in here, not narrated back by the
    // model: the conversation so far goes up as messages and the turns
    // since the last push are cited. A failure is said on the visible
    // line and the call goes through unchanged — the server then asks
    // for what is missing.
    let mut notes: Vec<String> = Vec::new();
    if payload
        .tool_name
        .as_deref()
        .is_some_and(|tool| tool.ends_with("decision_add"))
    {
        if let Ok(State::Bound { project, .. }) = marker::find(&payload.cwd) {
            match tokio::time::timeout(CTX_BUDGET, push_transcript(harness, &payload, project))
                .await
            {
                Ok(Ok(ids)) => cite(&mut merged, &ids),
                Ok(Err(e)) => notes.push(format!("evidence not recorded — {e}")),
                Err(_) => notes.push(format!(
                    "evidence not recorded — sync exceeded {CTX_BUDGET:?}"
                )),
            }
        }
        // Code citations arrive complete or not at all: this build carries
        // no resolver for a bare `path:lines`, so those are dropped and said.
        let dropped = drop_incomplete_code(&mut merged);
        if dropped > 0 {
            notes.push(format!(
                "{dropped} code citation(s) dropped — cite committed anchors in full"
            ));
        }
    }
    let system = (!notes.is_empty()).then(|| format!("Converge: {}", notes.join("; ")));

    respond(
        harness,
        Response::Ctx {
            tool_input: merged,
            system,
        },
    );
    Ok(())
}

/// Cite the newest of `ids` on the call, merged with what the model
/// cited itself, without duplicates.
fn cite(merged: &mut Value, ids: &[MessageId]) {
    let mut evidence: Vec<String> = merged["evidence"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let newest = ids.len().saturating_sub(CITED_TURNS);
    for id in &ids[newest..] {
        let id = id.to_string();
        if !evidence.contains(&id) {
            evidence.push(id);
        }
    }
    if !evidence.is_empty() {
        merged["evidence"] = json!(evidence);
    }
}

/// Keep only code anchors that carry every field; return how many went.
/// The seam for the resolver that completes a bare `path:lines` from
/// HEAD — see docs/tasks/hook-code-anchor-resolver.md.
fn drop_incomplete_code(merged: &mut Value) -> usize {
    let Some(items) = merged["code_evidence"].as_array_mut() else {
        return 0;
    };
    let before = items.len();
    items.retain(|a| {
        ["commit", "path", "lines", "excerpt", "digest"]
            .iter()
            .all(|k| !a[k].is_null())
    });
    let dropped = before - items.len();
    if items.is_empty() {
        merged.as_object_mut().map(|o| o.remove("code_evidence"));
    }
    dropped
}

pub(crate) fn remote(cwd: &Path) -> Option<String> {
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

    fn open(id: &str, tier: Tier, title: &str, new: bool) -> Open {
        Open {
            id: id.parse().unwrap(),
            tier,
            kind: "dependency".into(),
            title: title.into(),
            new,
        }
    }

    #[test]
    fn render_marks_what_the_user_was_never_shown() {
        let project: ProjectId = "01J00000000000000000000000".parse().unwrap();
        let lo = open("01J00000000000000000000001", Tier::Conflict, "lo", false);
        let mid = open("01J00000000000000000000002", Tier::Watch, "mid", false);
        let hi = open("01J00000000000000000000003", Tier::Coordinate, "hi", true);
        let signals = [lo.clone(), mid.clone(), hi.clone()];
        let decisions = ["- one [accepted]".to_string()];

        // Only `hi` has no receipt: it is new and leads; the conflict
        // leads the rest; the visible line counts both.
        let (block, system) = render(project, "p", &decisions, &signals);
        let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- [")).collect();
        assert_eq!(
            lines,
            [
                format!("- [coordinate/dependency] hi ({}) ← NEW", hi.id),
                format!("- [conflict/dependency] lo ({})", lo.id),
                format!("- [watch/dependency] mid ({})", mid.id),
            ]
        );
        assert!(block.contains("← NEW = not shown to you before"), "{block}");
        assert_eq!(
            system,
            "Converge: \"p\" — 1 decision(s), 3 open signal(s) (1 new, 1 conflict) ✓"
        );

        // Everything receipted: nothing is new, no legend, and the order
        // is tier then newest.
        let seen: Vec<Open> = signals
            .iter()
            .cloned()
            .map(|s| Open { new: false, ..s })
            .collect();
        let (block, system) = render(project, "p", &decisions, &seen);
        assert!(!block.contains("NEW"), "{block}");
        let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- [")).collect();
        assert!(
            lines[0].contains(" lo ") && lines[1].contains(" hi ") && lines[2].contains(" mid ")
        );
        assert_eq!(
            system,
            "Converge: \"p\" — 1 decision(s), 3 open signal(s) (1 conflict) ✓"
        );

        // Nothing open: no signal section, no parenthetical.
        let (block, system) = render(project, "p", &decisions, &[]);
        assert!(!block.contains("Proposed signals"));
        assert_eq!(
            system,
            "Converge: \"p\" — 1 decision(s), 0 open signal(s) ✓"
        );
    }

    fn arrived(id: &str, tier: Tier, title: &str, recommendation: Option<&str>) -> Signal {
        let decision = |s: &str| s.parse::<converge_client::DecisionId>().unwrap();
        Signal {
            id: id.parse().unwrap(),
            source: decision("01J00000000000000000000000"),
            targets: vec![decision("01J00000000000000000000001")],
            kind: "dependency".into(),
            tier,
            status: SignalStatus::Proposed,
            title: title.into(),
            text: format!("{title} bears on the other one.\n"),
            consequence: None,
            recommendation: recommendation.map(str::to_owned),
            produced_by: converge_client::Author::User(
                "01J00000000000000000000002".parse().unwrap(),
            ),
            resolved_by: None,
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn frame_says_what_arrived_and_whose_words_they_are() {
        let one = arrived("01J00000000000000000000003", Tier::Coordinate, "one", None);
        let (context, system) = frame(std::slice::from_ref(&one));
        assert_eq!(system, "Converge: 1 new signal");
        assert!(context.starts_with("Converge: 1 signal raised since your last prompt"));
        assert!(context.contains("not instructions"), "{context}");
        assert!(!context.contains("conflict-tier"), "{context}");
        assert!(
            context.contains(&format!(
                "- [coordinate/dependency] one ({}): one bears on the other one.",
                one.id
            )),
            "{context}"
        );
        assert!(context.ends_with("never your own."), "{context}");

        let two = arrived(
            "01J00000000000000000000004",
            Tier::Conflict,
            "two",
            Some(" talk to billing "),
        );
        let (context, system) = frame(&[one, two]);
        assert_eq!(system, "Converge: 2 new signals (1 conflict)");
        assert!(
            context.contains("put it to the user before continuing"),
            "{context}"
        );
        assert!(
            context.contains("\n  Recommendation: talk to billing\n"),
            "{context}"
        );
    }

    #[test]
    fn evidence_is_cited_newest_first_and_incomplete_code_is_dropped() {
        let ids: Vec<MessageId> = (0..12).map(|_| MessageId::new()).collect();
        let mut call = json!({ "title": "t", "evidence": [ids[11].to_string(), "01J00000000000000000000009"] });
        cite(&mut call, &ids);
        let cited: Vec<&str> = call["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        // The model's own citations stay first; the newest ten follow,
        // the one already cited not twice.
        assert_eq!(cited.len(), 2 + CITED_TURNS - 1);
        assert_eq!(cited[0], ids[11].to_string());
        assert!(!cited.contains(&ids[0].to_string().as_str()));
        assert!(cited.contains(&ids[2].to_string().as_str()));

        let mut call = json!({ "code_evidence": [
            { "path": "a.rs", "lines": [1, 2] },
            { "commit": "c", "path": "b.rs", "lines": [1, 1], "excerpt": "x", "digest": "d" },
        ]});
        assert_eq!(drop_incomplete_code(&mut call), 1);
        assert_eq!(call["code_evidence"].as_array().unwrap().len(), 1);
        let mut call = json!({ "code_evidence": [{ "path": "a.rs" }] });
        assert_eq!(drop_incomplete_code(&mut call), 1);
        assert!(call.get("code_evidence").is_none());
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
