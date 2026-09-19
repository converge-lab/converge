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
//!   the LLM gathering anything. Before a `decision_add` it goes further:
//!   the conversation goes up as evidence and the citations the model
//!   made — message turns and bare `path:lines` code — are completed
//!   here, where the working tree and its repository can be read.
//! - `mark` (post-tool, matched on the binding tools): perform the
//!   **local effect** — parse the tool response and write the marker at
//!   the git root. The LLM only ever chose; the write is deterministic.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use converge_client::{
    DecisionFilter, DecisionId, MessageId, Pagination, ProjectId, Signal, SignalFilter, SignalId,
    SignalStatus, Tier,
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
type Index = Option<(String, Vec<Recorded>, Vec<Open>)>;

/// A decision as the block lists it.
#[derive(Debug, Clone, PartialEq)]
struct Recorded {
    id: DecisionId,
    title: String,
    status: String,
    /// Never shown to this user before, in any session on any machine.
    new: bool,
}

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

/// The decision lines: what is new to this reader leads, newest first
/// within each half, so a session start opens on what changed.
fn lines_of_decisions(decisions: &[Recorded]) -> String {
    let mut shown: Vec<&Recorded> = decisions.iter().collect();
    shown.sort_by_key(|d| (std::cmp::Reverse(d.new), std::cmp::Reverse(d.id)));
    shown
        .iter()
        .map(|d| {
            format!(
                "- {} [{}]{}",
                d.title,
                d.status,
                if d.new { " ← NEW" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The bound block and its visible line, from what the server said.
/// Pure, so the wording and the order are testable without a server.
/// This listing is deliberately unfiltered by receipts: it is the one
/// place every open signal shows, whatever a poll handed out or dropped.
fn render(
    project: ProjectId,
    name: &str,
    decisions: &[Recorded],
    signals: &[Open],
) -> (String, String) {
    let is_new = |s: &Open| s.new;
    let new = signals.iter().filter(|s| is_new(s)).count();
    let new_decisions = decisions.iter().filter(|d| d.new).count();
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
        "Converge: \"{name}\" — {} decision(s){}, {} open signal(s){detail} ✓",
        counted(decisions.len(), 30),
        match new_decisions {
            0 => String::new(),
            n => format!(" ({n} new)"),
        },
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
             when a new decision lands.\n\nDecisions{}:\n{}",
            if new_decisions > 0 {
                " (← NEW = not shown to you before, in any session)"
            } else {
                ""
            },
            lines_of_decisions(decisions),
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
        // Both lists, and for each the half this user has never been
        // shown: four reads in one round trip's worth of waiting, and
        // the marks fall out of the difference rather than any local
        // file, so they are the same on every machine.
        let recorded = DecisionFilter {
            project: Some(project),
            ..Default::default()
        };
        let recorded_unseen = DecisionFilter {
            unseen: true,
            ..recorded.clone()
        };
        let open = SignalFilter {
            project: Some(project),
            status: Some(SignalStatus::Proposed),
            ..Default::default()
        };
        let open_unseen = SignalFilter {
            unseen: true,
            ..open.clone()
        };
        let thirty = Pagination {
            limit: Some(30),
            cursor: None,
        };
        let ten = Pagination {
            limit: Some(10),
            cursor: None,
        };
        let (all_decisions, new_decisions, listed, fresh) = tokio::try_join!(
            client.decision_list(&recorded, &thirty),
            client.decision_list(&recorded_unseen, &thirty),
            client.signal_list(&open, &ten),
            client.signal_list(&open_unseen, &ten)
        )?;
        let new_decisions: BTreeSet<DecisionId> =
            new_decisions.items.iter().map(|d| d.id).collect();
        let decisions: Vec<Recorded> = all_decisions
            .items
            .iter()
            .map(|d| Recorded {
                id: d.id,
                title: d.title.clone(),
                status: format!("{:?}", d.status).to_lowercase(),
                new: new_decisions.contains(&d.id),
            })
            .collect();
        let fresh: BTreeSet<SignalId> = fresh.items.iter().map(|s| s.id).collect();
        let signals: Vec<Open> = listed
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
    let mut listed_decisions: Vec<DecisionId> = Vec::new();
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
            listed_decisions = decisions.iter().map(|d| d.id).collect();
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
            client.receive(session, Some(harness), &listed, &listed_decisions),
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
    let Some(session) = payload.session.clone().filter(|s| !s.trim().is_empty()) else {
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
    // The same gate that paces the poll paces the evidence backlog: a
    // bounded, oldest-first pass in a process of its own, so a decision
    // recorded later in this session finds its conversation already on
    // the server and has almost nothing left to send.
    spawn_drain(kind, &payload);
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

/// Start a drain for this session's transcript and return at once: the
/// prompt waits for nothing, and a drain already running keeps its
/// lock. Nothing here is reported — a backlog that cannot be sent is
/// the session-start block's problem, not this turn's.
fn spawn_drain(kind: Kind, payload: &Payload) {
    let Some(transcript) = payload.transcript.as_ref() else {
        return;
    };
    if crate::drain::locked(&transcript.key()) {
        return;
    }
    let named = match transcript {
        crate::harness::Transcript::File(path) => path.to_string_lossy().into_owned(),
        crate::harness::Transcript::Session(id) => id.clone(),
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::process::Stdio;
    let mut drain = Command::new(exe);
    drain
        .args(["hook", "drain", "--harness", kind.flag()])
        .arg("--cwd")
        .arg(&payload.cwd)
        .arg("--transcript")
        .arg(named)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: the drain outlives the prompt that
        // started it, the way the self-update does.
        drain.process_group(0);
    }
    let _ = drain.spawn();
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
    if let Err(e) = try_sync(kind).await {
        respond(
            harness,
            Response::Notice {
                system: format!("Converge: sync skipped — {e}"),
            },
        );
    }
    Ok(())
}

async fn try_sync(kind: Kind) -> Result<()> {
    let harness = kind.harness();
    let payload = harness.parse(&raw());
    // Only bound repos sync; unbound and disabled stay quiet.
    let State::Bound { project, .. } = marker::find(&payload.cwd)? else {
        return Ok(());
    };
    let (ids, behind) = record_conversation(kind, &payload, project).await?;
    let added = ids.len();
    if added > 0 {
        respond(
            harness,
            Response::Notice {
                system: match behind {
                    0 => format!("Converge: synced {added} message(s) to evidence ✓"),
                    n => format!(
                        "Converge: synced {added} message(s) to evidence ✓ \
                         ({n} still to send)"
                    ),
                },
            },
        );
    }
    Ok(())
}

/// Put the conversation on the server and answer with the ids a
/// decision can cite, plus how many turns are still waiting.
///
/// A citation has to name messages that exist, so this never hands back
/// nothing while the session holds anything at all: it sends what it
/// can and then cites, falling back to the newest turns already
/// recorded. Sending is bounded twice over — one batch, and a slice of
/// the hook's budget — because the per-prompt drain is what empties a
/// backlog; this only has to cover the turns since the last prompt.
///
/// One writer at a time: the same lock a drain takes, waited on
/// briefly, since a pass releases it between batches. Turns always go
/// oldest first, so the stored conversation reads in order.
async fn record_conversation(
    kind: Kind,
    payload: &Payload,
    project: ProjectId,
) -> Result<(Vec<MessageId>, usize)> {
    let Some(at) = payload.transcript.as_ref() else {
        return Ok((Vec::new(), 0));
    };
    if let Some(lock) = crate::drain::Lock::take_within(&at.key(), LOCK_WAIT) {
        let sent = tokio::time::timeout(
            PUSH_BUDGET,
            crate::drain::pass(kind, &payload.cwd, at, PUSH_CAP, lock),
        )
        .await;
        // A decision cites the exchange that produced it, which is the
        // last few turns of the session — not only whichever of them
        // happened to be unsent. Fresh ids are enough on their own only
        // when there are enough of them; otherwise the read below picks
        // up what was just sent along with what was already there.
        if let Ok(Ok((ids, behind))) = sent
            && ids.len() >= CITED_TURNS
        {
            return Ok((ids, behind));
        }
    }
    cite_recorded(kind, payload, project).await
}

/// The newest turns the server already has for this session, and how
/// many are still unsent. Used when there was nothing new to send, when
/// sending ran long, and when a drain holds the lock.
async fn cite_recorded(
    kind: Kind,
    payload: &Payload,
    project: ProjectId,
) -> Result<(Vec<MessageId>, usize)> {
    let Some(at) = payload.transcript.as_ref() else {
        return Ok((Vec::new(), 0));
    };
    let harness = kind.harness();
    let parsed = harness.transcript(at)?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok((Vec::new(), 0));
    };
    let client = Config::load()?.client()?;
    let session = client
        .session_ensure(&converge_client::NewSession {
            project_id: project,
            kind: converge_client::SessionKind::Transcript,
            external,
            title: crate::transcript::title(&parsed.turns, &format!("{} session", harness.label())),
        })
        .await?;
    let recorded = client
        .message_list(
            session,
            &Pagination {
                limit: None,
                cursor: None,
            },
        )
        .await?;
    let tail = recorded.items.len().saturating_sub(CITED_TURNS);
    let ids: Vec<MessageId> = recorded.items[tail..].iter().map(|m| m.id).collect();
    let behind = crate::watermark::Watermarks::load()?
        .pending(&at.key(), &parsed.turns)
        .len();
    Ok((ids, behind))
}

// ─── pre-tool: context collector ──────────────────────────────────────────

/// The budget for putting the conversation on record before a
/// `decision_add`: a few turns to the server. Past it, the call goes
/// through as the model made it and the server says what is missing.
const CTX_BUDGET: std::time::Duration = std::time::Duration::from_secs(4);
/// How many of the turns since the last push a decision cites.
const CITED_TURNS: usize = 10;
/// How many turns one hook-driven pass carries. The per-prompt drain
/// keeps the backlog near zero, so this is the catch-up case: send a
/// batch, cite it, and let the drain take the rest.
const PUSH_CAP: usize = 50;
/// How long the hook waits for a running drain to release the lock. A
/// drain releases between batches, so this is usually not spent.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(900);
/// The slice of the hook's budget sending may take. What is left over
/// is what reads back the citation, so a slow send costs relevance,
/// never the decision.
const PUSH_BUDGET: std::time::Duration = std::time::Duration::from_millis(2200);

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
            match tokio::time::timeout(CTX_BUDGET, record_conversation(kind, &payload, project))
                .await
            {
                Ok(Ok((ids, behind))) => {
                    cite(&mut merged, &ids);
                    if behind > 0 {
                        notes.push(format!(
                            "evidence lags by {behind} turn(s), still being recorded"
                        ));
                    }
                }
                Ok(Err(e)) => notes.push(format!("evidence not recorded — {e}")),
                Err(_) => notes.push(format!(
                    "evidence not recorded — sync exceeded {CTX_BUDGET:?}"
                )),
            }
        }
        // Code citations are filled in here too: a bare `path:lines` is
        // completed from the repository under the working tree — commit,
        // excerpt, digest — and what the repository cannot vouch for is
        // refused, and said, rather than dropped in silence.
        let refusals = crate::evidence::complete(&mut merged, &payload.cwd);
        let dropped = drop_incomplete_code(&mut merged);
        notes.extend(refusals);
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
/// [`crate::evidence`] completes a bare `path:lines` from HEAD; this is
/// the last line behind it. An item that names no path or no lines — or
/// names a commit and nothing else — is a half anchor the server would
/// reject whole, so it goes, and the visible line says so.
fn drop_incomplete_code(merged: &mut Value) -> usize {
    // `get_mut`, not `merged[..]`: indexing a `Value` *inserts* a null,
    // and an absent `code_evidence` must stay absent.
    let Some(items) = merged
        .get_mut("code_evidence")
        .and_then(Value::as_array_mut)
    else {
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

    fn recorded(id: &str, title: &str, new: bool) -> Recorded {
        Recorded {
            id: id.parse().unwrap(),
            title: title.into(),
            status: "accepted".into(),
            new,
        }
    }

    #[test]
    fn render_marks_decisions_the_user_was_never_shown() {
        let project: ProjectId = "01J00000000000000000000000".parse().unwrap();
        let old = recorded("01J0000000000000000000000A", "settled last week", false);
        let fresh = recorded("01J0000000000000000000000B", "settled today", true);
        let decisions = [old, fresh];

        let (block, system) = render(project, "p", &decisions, &[]);
        let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- ")).collect();
        // What this reader has not seen leads, and says so once.
        assert_eq!(
            lines,
            [
                "- settled today [accepted] ← NEW",
                "- settled last week [accepted]"
            ]
        );
        assert!(
            block.contains("Decisions (← NEW = not shown to you before"),
            "{block}"
        );
        assert_eq!(
            system,
            "Converge: \"p\" — 2 decision(s) (1 new), 0 open signal(s) ✓"
        );

        // Nothing new: no mark, no legend, no parenthetical.
        let seen: Vec<Recorded> = decisions
            .iter()
            .cloned()
            .map(|d| Recorded { new: false, ..d })
            .collect();
        let (block, system) = render(project, "p", &seen, &[]);
        assert!(!block.contains("NEW"), "{block}");
        assert_eq!(
            system,
            "Converge: \"p\" — 2 decision(s), 0 open signal(s) ✓"
        );
    }

    #[test]
    fn render_marks_what_the_user_was_never_shown() {
        let project: ProjectId = "01J00000000000000000000000".parse().unwrap();
        let lo = open("01J00000000000000000000001", Tier::Conflict, "lo", false);
        let mid = open("01J00000000000000000000002", Tier::Watch, "mid", false);
        let hi = open("01J00000000000000000000003", Tier::Coordinate, "hi", true);
        let signals = [lo.clone(), mid.clone(), hi.clone()];
        let decisions = [recorded("01J0000000000000000000000A", "one", false)];

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
        // And an absent list stays absent — not a null in its place.
        let mut call = json!({ "title": "t" });
        assert_eq!(drop_incomplete_code(&mut call), 0);
        assert!(call.get("code_evidence").is_none(), "{call}");
    }

    #[test]
    fn a_bare_citation_and_a_full_anchor_arrive_as_two_anchors() {
        use converge_client::CodeAnchor;
        let repo = crate::evidence::test_repo(&[("src/lib.rs", "one\ntwo\n")]);
        let whole = json!({
            "commit": "0".repeat(40), "path": "other.rs", "lines": [3, 4],
            "excerpt": "x\ny\n", "digest": CodeAnchor::digest_of("x\ny\n"),
        });
        let mut call = json!({ "code_evidence": [
            { "path": "src/lib.rs", "lines": [1, 2] },
            whole,
        ]});
        // The bare citation is filled from HEAD; the complete one is left
        // alone. Nothing is refused, so nothing is said on the line.
        assert!(crate::evidence::complete(&mut call, &repo).is_empty());
        assert_eq!(drop_incomplete_code(&mut call), 0);
        let kept = call["code_evidence"].as_array().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0]["path"], "src/lib.rs");
        assert_eq!(kept[0]["lines"], json!([1, 2]));
        assert_eq!(kept[0]["excerpt"], "one\ntwo\n");
        assert_eq!(kept[0]["digest"], CodeAnchor::digest_of("one\ntwo\n"));
        assert_eq!(kept[0]["commit"].as_str().unwrap().len(), 40);
        assert_eq!(kept[1]["path"], "other.rs");
        assert_eq!(kept[1]["commit"], "0".repeat(40));
        std::fs::remove_dir_all(&repo).ok();
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
