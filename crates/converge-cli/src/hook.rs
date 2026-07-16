//! The hook entrypoints — Claude Code invokes these; they never prompt
//! and they never fail the session (best-effort output, always exit 0
//! from `run`).
//!
//! Ported from the validated POC (`poc-mapping`): the inject rules
//! wording is contract-like — agents act on it — so it changes carefully.
//!
//! - `inject` (SessionStart): read the three-state marker, emit the
//!   context block — bound (binding + decision index), disabled
//!   (stay-quiet rules), unbound (the mapping rules), or unreadable.
//! - `ctx` (PreToolUse, matched on converge tools): merge `cwd` + git
//!   remote into the tool input, so the server ranks candidates without
//!   the LLM gathering anything.
//! - `mark` (PostToolUse, matched on the binding tools): perform the
//!   **local effect** — parse the tool response and write the marker at
//!   the git root. The LLM only ever chose; the write is deterministic.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use converge_client::{DecisionFilter, Pagination, ProjectId};
use serde_json::{Value, json};

use crate::config::Config;
use crate::marker::{self, State};

/// The stdin payload Claude Code hands every hook (fields we use).
fn payload() -> Value {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        return Value::Null;
    }
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn cwd_of(payload: &Value) -> PathBuf {
    payload["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn emit(value: &Value) {
    println!("{value}");
}

// ─── SessionStart ───────────────────────────────────────────────────────────

pub async fn inject() -> Result<()> {
    let payload = payload();
    let cwd = cwd_of(&payload);

    let (context, system) = match marker::find(&cwd) {
        Ok(State::Bound { project, .. }) => (
            bound(project).await,
            format!("Converge: bound to {project} ✓"),
        ),
        Ok(State::Disabled { .. }) => (
            "## Converge — disabled for this repo\n\
             Converge is off here (`.converge` has `disable = true`). Do NOT \
             suggest mapping. To re-enable, remove that line from `.converge` \
             (or delete the file)."
                .to_string(),
            "Converge: disabled for this repo".to_string(),
        ),
        Ok(State::Unbound) => (
            UNBOUND.to_string(),
            "Converge: repo unmapped — link it via project_match".to_string(),
        ),
        Err(_) => (
            "## Converge — marker unreadable\n\
             `.converge` exists but has neither `project_id` nor `disable`. \
             Re-link: call `project_match`, then `project_bind` — or tell \
             the user to run `converge project init --rebind`."
                .to_string(),
            "Converge: marker unreadable — re-link".to_string(),
        ),
    };

    emit(&json!({
        "systemMessage": system,
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context,
        },
    }));
    Ok(())
}

/// The unbound rules — the POC's tested wording, tool names updated to
/// the `resource_operation` palette.
const UNBOUND: &str = "## Converge — this repo is UNMAPPED\n\
There is no `.converge` marker, so project memory is unavailable until \
this working tree is linked to a converge project.\n\n\
**Do this now, proactively, without being asked:**\n\
1. Call `project_match` (no arguments — a hook fills in the context).\n\
2. If the response carries an outcome (`project_id` or `disable: \
true`), the server already asked the user and a hook writes the marker \
— you are done; do NOT render your own menu.\n\
3. If it carries `candidates`: present them via `AskUserQuestion` (one \
per candidate + a 'Disable Converge for this repo' option; tell the \
user the built-in 'Type something' is MANUAL MAPPING — an existing id \
links, a new name creates), then `project_bind` with the pick, or \
`project_dismiss` scope='repo' to disable.\n\n\
Do NOT write `.converge` yourself — the hooks do it. Start with step 1 \
right away.";

/// The bound block: binding + a compact decision index, fetched
/// best-effort (a hook must not fail the session because the server is
/// down — the binding itself is local knowledge).
async fn bound(project: ProjectId) -> String {
    let fetched: Result<(String, Vec<String>)> = async {
        let config = Config::load()?;
        let client = config.client()?;
        let name = client
            .project_get(project)
            .await?
            .map(|p| p.name)
            .unwrap_or_else(|| project.to_string());
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
        Ok((name, decisions))
    }
    .await;

    // The daily-cached skew nudge rides the bound path (the one that
    // already talks to the server); its failure is silence, not noise.
    let skew = match Config::load().ok().and_then(|c| c.client().ok()) {
        Some(client) => crate::skew::check_cached(&client).await,
        None => None,
    };
    let block = match fetched {
        Ok((name, decisions)) if decisions.is_empty() => format!(
            "## Converge memory — project \"{name}\" ({project})\n\
             This working tree is bound to converge project `{project}`; \
             project memory is active. No decisions are recorded yet — use \
             `decision_add` when a design decision lands, and record the \
             conversation (`session_ensure` + `message_add`) so decisions \
             can cite their evidence."
        ),
        Ok((name, decisions)) => format!(
            "## Converge memory — project \"{name}\" ({project})\n\
             This working tree is bound to converge project `{project}`; \
             project memory is active. Decisions below are in force — \
             `decision_get` for the full record before re-deciding a \
             settled topic; `decision_add` (with `supersedes`/`evidence`) \
             when a new decision lands.\n\nDecisions:\n{}",
            decisions.join("\n")
        ),
        Err(_) => format!(
            "## Converge memory — project {project}\n\
             This working tree is bound to converge project `{project}`, \
             but the server was unreachable when this session started — \
             the decision index is unavailable. Tools may still work; \
             `decision_list` fetches the index on demand."
        ),
    };
    match skew {
        Some(warning) => format!("{block}\n\n(note: {warning})"),
        None => block,
    }
}

// ─── SessionEnd: transcript → evidence ──────────────────────────────────────

pub async fn sync() -> Result<()> {
    // Best effort throughout: a sync problem must never surface as a
    // session failure. The quiet paths just return.
    if let Err(e) = try_sync().await {
        emit(&json!({ "systemMessage": format!("Converge: sync skipped — {e}") }));
    }
    Ok(())
}

async fn try_sync() -> Result<()> {
    let payload = payload();
    let cwd = cwd_of(&payload);
    let Some(transcript) = payload["transcript_path"].as_str() else {
        return Ok(());
    };

    // Only bound repos sync; unbound and disabled stay quiet.
    let State::Bound { project, .. } = marker::find(&cwd)? else {
        return Ok(());
    };

    let parsed = crate::transcript::read(std::path::Path::new(transcript))?;
    let Some(external) = parsed.session_id.clone() else {
        return Ok(()); // no session id in the content — nothing to key on
    };

    let mut marks = crate::watermark::Watermarks::load()?;
    let already = marks.synced(transcript);
    // A shrunk/rewritten transcript (fewer turns than synced) is left
    // alone rather than re-sent, to avoid duplicating evidence.
    let Some(fresh) = parsed.turns.get(already..).filter(|f| !f.is_empty()) else {
        marks.set(transcript, parsed.turns.len());
        marks.save()?;
        return Ok(());
    };

    let config = Config::load()?;
    let client = config.client()?;

    let session = client
        .session_ensure(&converge_client::NewSession {
            project_id: project,
            kind: converge_client::SessionKind::Transcript,
            external,
            title: crate::transcript::title(&parsed.turns),
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

    marks.set(transcript, parsed.turns.len());
    marks.save()?;
    emit(&json!({
        "systemMessage": format!("Converge: synced {added} message(s) to evidence ✓"),
    }));
    Ok(())
}

// ─── PreToolUse: context collector ──────────────────────────────────────────

pub fn ctx() -> Result<()> {
    let payload = payload();
    let cwd = cwd_of(&payload);

    let mut merged = payload["tool_input"].clone();
    if !merged.is_object() {
        merged = json!({});
    }
    merged["cwd"] = json!(cwd.to_string_lossy());
    if let Some(remote) = remote(&cwd) {
        merged["remote"] = json!(remote);
    }

    emit(&json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "updatedInput": merged,
        },
    }));
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

// ─── PostToolUse: the local effect ──────────────────────────────────────────

pub fn mark() -> Result<()> {
    let payload = payload();
    let cwd = cwd_of(&payload);
    let root = marker::root(&cwd);

    let response = tool_json(&payload["tool_response"]);
    let message = match effect(&response, &root) {
        Effect::Bound(name) => {
            format!("Converge: linked this repo to \"{name}\" (wrote .converge) ✓")
        }
        Effect::Disabled => "Converge: disabled for this repo (wrote .converge)".to_string(),
        Effect::Nothing => {
            // Session-scoped dismiss, a skip, or an unrecognized payload:
            // deliberately no local effect and no noise.
            return Ok(());
        }
        Effect::Failed(err) => format!("Converge: could not write .converge — {err}"),
    };
    emit(&json!({ "systemMessage": message }));
    Ok(())
}

enum Effect {
    Bound(String),
    Disabled,
    Nothing,
    Failed(String),
}

/// Interpret a binding tool's response: `{project_id, name}` → bound,
/// `{disable: true}` → disabled, anything else → no local effect.
fn effect(response: &Value, root: &Path) -> Effect {
    if response["disable"].as_bool() == Some(true) {
        return match marker::write_disabled(root) {
            Ok(_) => Effect::Disabled,
            Err(e) => Effect::Failed(e.to_string()),
        };
    }
    if let Some(id) = response["project_id"].as_str() {
        let Ok(project) = id.parse() else {
            return Effect::Failed(format!("`{id}` is not a project id"));
        };
        let name = response["name"].as_str().unwrap_or(id).to_string();
        return match marker::write_bound(root, project, &name) {
            Ok(_) => Effect::Bound(name),
            Err(e) => Effect::Failed(e.to_string()),
        };
    }
    Effect::Nothing
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
        assert!(matches!(effect(&response, &root), Effect::Bound(n) if n == "gw"));
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
        assert!(matches!(effect(&response, &root), Effect::Bound(n) if n == "gw"));

        // Dismiss repo → disabled (overwrites).
        let response = tool_json(&json!({ "dismissed": "repo", "disable": true }));
        assert!(matches!(effect(&response, &root), Effect::Disabled));
        assert!(matches!(
            marker::find(&root).unwrap(),
            State::Disabled { .. }
        ));

        // Session-scope dismiss and skips: no local effect.
        for benign in [
            json!({ "dismissed": "session", "disable": false }),
            json!({ "skipped": true }),
            json!({ "elicitation": false }),
            Value::Null,
        ] {
            assert!(matches!(effect(&benign, &root), Effect::Nothing));
        }

        // Garbage id fails loudly, writes nothing over the disabled state.
        let response = json!({ "project_id": "nonsense" });
        assert!(matches!(effect(&response, &root), Effect::Failed(_)));
        assert!(matches!(
            marker::find(&root).unwrap(),
            State::Disabled { .. }
        ));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
