//! Parsing recorded conversations into evidence turns.
//!
//! One parser per transcript format, named for the harness that writes
//! it; [`Parsed`] and [`Turn`] are what they all produce.
//!
//! **Claude Code** — [`claude`]:
//!
//! Each line is a JSON record; only `user`/`assistant` entries with
//! visible text become turns (tool calls, tool results, and thinking are
//! conversation noise for evidence). `sessionId` and `cwd` are read from
//! the content — the filename is unreliable (subagent transcripts share
//! the parent id).
//!
//! The whole file is read each sync and turns are deduplicated by count
//! (append-only transcripts only grow), which keeps the derived session
//! title stable across incremental syncs.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// One conversation turn, ready to become a `NewMessage`.
pub struct Turn {
    pub speaker: String,
    pub body: String,
    pub sent_at: Option<OffsetDateTime>,
}

/// A parsed transcript.
#[derive(Default)]
pub struct Parsed {
    /// The session's own id (from content) — the evidence natural key.
    pub session_id: Option<String>,
    /// First working directory seen — resolves the project.
    pub cwd: Option<String>,
    /// The conversation turns in file order.
    pub turns: Vec<Turn>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<Content>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Parse a whole Claude Code transcript. Unparseable lines are skipped
/// (a partial trailing line from a concurrent write is simply left for
/// next time).
pub fn claude(path: &Path) -> Result<Parsed> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut parsed = Parsed::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<Line>(line) else {
            continue;
        };
        let speaker = match raw.kind.as_deref() {
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => continue,
        };
        if parsed.session_id.is_none() {
            parsed.session_id = raw.session_id.clone();
        }
        if parsed.cwd.is_none() {
            parsed.cwd = raw.cwd.clone();
        }
        let body = raw
            .message
            .as_ref()
            .and_then(|m| m.content.as_ref())
            .map(flatten)
            .unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        parsed.turns.push(Turn {
            speaker: speaker.into(),
            body,
            sent_at: raw
                .timestamp
                .as_deref()
                .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok()),
        });
    }
    Ok(parsed)
}

// ─── Codex CLI: rollout JSONL ────────────────────────────────────────

/// Codex records a rollout: a `session_meta` header line carrying the
/// session id and cwd, then `response_item` lines of which only
/// `message` payloads are conversation.
#[derive(Deserialize)]
struct Rollout {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    payload: Option<RolloutPayload>,
}

#[derive(Deserialize)]
struct RolloutPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    // `session_meta`
    session_id: Option<String>,
    cwd: Option<String>,
    // `message`
    role: Option<String>,
    #[serde(default)]
    content: Vec<RolloutBlock>,
}

#[derive(Deserialize)]
struct RolloutBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Blocks Codex injects into the conversation as if a human had typed
/// them. `developer` messages are all scaffolding and drop wholesale;
/// these arrive wearing the `user` role, so they are matched by name.
/// An unknown future block leaks into evidence rather than swallowing a
/// human turn — the right way round to be wrong.
const SCAFFOLDING: [&str; 4] = [
    "<environment_context>",
    "<skills_instructions>",
    "<user_instructions>",
    "<turn_aborted>",
];

/// Parse a whole Codex rollout. As with Claude, unparseable lines are
/// skipped rather than fatal.
pub fn codex(path: &Path) -> Result<Parsed> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut parsed = Parsed::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<Rollout>(line) else {
            continue;
        };
        let Some(payload) = raw.payload else {
            continue;
        };
        match (raw.kind.as_deref(), payload.kind.as_deref()) {
            (Some("session_meta"), _) => {
                if parsed.session_id.is_none() {
                    parsed.session_id = payload.session_id;
                }
                if parsed.cwd.is_none() {
                    parsed.cwd = payload.cwd;
                }
            }
            (Some("response_item"), Some("message")) => {
                // `developer` is the harness talking to the model.
                let speaker = match payload.role.as_deref() {
                    Some("user") => "user",
                    Some("assistant") => "assistant",
                    _ => continue,
                };
                let body = payload
                    .content
                    .iter()
                    .filter(|b| b.kind == "input_text" || b.kind == "output_text")
                    .filter_map(|b| b.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let trimmed = body.trim();
                if trimmed.is_empty() || SCAFFOLDING.iter().any(|tag| trimmed.starts_with(tag)) {
                    continue;
                }
                parsed.turns.push(Turn {
                    speaker: speaker.into(),
                    body,
                    sent_at: raw
                        .timestamp
                        .as_deref()
                        .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok()),
                });
            }
            _ => continue,
        }
    }
    Ok(parsed)
}

// ─── opencode: `opencode export` JSON ──────────────────────────────────

/// What `opencode export <session>` prints: session info, then messages
/// whose visible prose lives in `text` parts.
#[derive(Deserialize)]
struct Export {
    info: Option<ExportInfo>,
    #[serde(default)]
    messages: Vec<ExportMessage>,
}

#[derive(Deserialize)]
struct ExportInfo {
    id: Option<String>,
    directory: Option<String>,
}

#[derive(Deserialize)]
struct ExportMessage {
    info: Option<ExportMessageInfo>,
    #[serde(default)]
    parts: Vec<ExportPart>,
}

#[derive(Deserialize)]
struct ExportMessageInfo {
    role: Option<String>,
    time: Option<ExportTime>,
}

#[derive(Deserialize)]
struct ExportTime {
    /// Milliseconds since the epoch — opencode's own clock format.
    created: Option<i128>,
}

#[derive(Deserialize)]
struct ExportPart {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Parse an `opencode export` payload. Unlike the JSONL formats this is
/// one document: a malformed one is an error rather than a skipped line,
/// because there is nothing partial to salvage.
pub fn opencode(json: &[u8]) -> Result<Parsed> {
    let export: Export =
        serde_json::from_slice(json).context("parse the `opencode export` payload")?;
    let mut parsed = Parsed::default();
    if let Some(info) = export.info {
        parsed.session_id = info.id;
        parsed.cwd = info.directory;
    }
    for message in export.messages {
        let info = message.info.unwrap_or(ExportMessageInfo {
            role: None,
            time: None,
        });
        let speaker = match info.role.as_deref() {
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => continue,
        };
        let body = message
            .parts
            .iter()
            .filter(|p| p.kind == "text")
            .filter_map(|p| p.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n\n");
        if body.trim().is_empty() {
            continue;
        }
        parsed.turns.push(Turn {
            speaker: speaker.into(),
            body,
            sent_at: info
                .time
                .and_then(|t| t.created)
                .and_then(|ms| OffsetDateTime::from_unix_timestamp_nanos(ms * 1_000_000).ok()),
        });
    }
    Ok(parsed)
}

// ─── shared ────────────────────────────────────────────────────────────

/// Visible prose only: `text` blocks (plain strings, or the text of a
/// blocks array). Tool calls, tool results, and thinking are dropped.
fn flatten(content: &Content) -> String {
    match content {
        Content::Text(s) => s.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// A short session title from the first user turn's first line;
/// `fallback` names the harness when there is no prose to take it from.
pub fn title(turns: &[Turn], fallback: &str) -> String {
    let first = turns
        .iter()
        .find(|t| t.speaker == "user")
        .and_then(|t| t.body.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        return fallback.to_string();
    }
    let capped: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        format!("{capped}…")
    } else {
        capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_visible_turns_only() {
        let path = std::env::temp_dir().join(format!("cvg-tx-{}.jsonl", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        // user string turn; assistant blocks turn with text + a tool_use
        // (dropped); a tool-result-only user turn (empty → skipped); one
        // unparseable line (skipped).
        writeln!(
            f,
            r#"{{"type":"user","sessionId":"s-1","cwd":"/repo","timestamp":"2026-07-12T10:00:00Z","message":{{"content":"split the trait?"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","timestamp":"2026-07-12T10:00:05Z","message":{{"content":[{{"type":"text","text":"yes — per-resource"}},{{"type":"tool_use","name":"x"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"…"}}]}}}}"#
        )
        .unwrap();
        writeln!(f, "not json").unwrap();
        f.flush().unwrap();

        let parsed = claude(&path).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("s-1"));
        assert_eq!(parsed.cwd.as_deref(), Some("/repo"));
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].speaker, "user");
        assert_eq!(parsed.turns[1].body, "yes — per-resource");
        assert!(parsed.turns[0].sent_at.is_some());
        assert_eq!(
            title(&parsed.turns, "Claude Code session"),
            "split the trait?"
        );
        assert_eq!(title(&[], "Claude Code session"), "Claude Code session");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn codex_rollout_keeps_prose_and_drops_scaffolding() {
        let path = std::env::temp_dir().join(format!("cvg-rollout-{}.jsonl", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        // Header; a developer block; an <environment_context> wearing the
        // user role; the one real question; the answer; a function_call
        // (not conversation); one unparseable line.
        for line in [
            r#"{"type":"session_meta","timestamp":"2026-09-16T14:25:46.754Z","payload":{"session_id":"01a0","cwd":"/repo","cli_version":"0.154.0"}}"#,
            r#"{"type":"response_item","timestamp":"2026-09-16T14:25:46.761Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<skills_instructions>\nuse skills\n</skills_instructions>"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-09-16T14:25:46.762Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-09-16T14:25:47.000Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"split the trait?"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-09-16T14:25:58.916Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"yes \u2014 per-resource"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-09-16T14:26:00.000Z","payload":{"type":"function_call","name":"shell"}}"#,
            "not json",
        ] {
            writeln!(f, "{line}").unwrap();
        }
        f.flush().unwrap();

        let parsed = codex(&path).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("01a0"));
        assert_eq!(parsed.cwd.as_deref(), Some("/repo"));
        assert_eq!(parsed.turns.len(), 2, "scaffolding leaked into evidence");
        assert_eq!(parsed.turns[0].speaker, "user");
        assert_eq!(parsed.turns[0].body, "split the trait?");
        assert_eq!(parsed.turns[1].speaker, "assistant");
        assert_eq!(parsed.turns[1].body, "yes \u{2014} per-resource");
        assert!(parsed.turns[0].sent_at.is_some());
        assert_eq!(
            title(&parsed.turns, "Codex CLI session"),
            "split the trait?"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn opencode_export_keeps_text_parts_only() {
        // The shape `opencode export` actually prints, trimmed.
        let json = br#"{
          "info": { "id": "ses_abc", "directory": "/repo", "title": "Greeting" },
          "messages": [
            { "info": { "role": "user", "time": { "created": 1789593180822 } },
              "parts": [ { "type": "text", "text": "split the trait?" } ] },
            { "info": { "role": "assistant", "time": { "created": 1789593181108 } },
              "parts": [ { "type": "reasoning", "text": "hmm" },
                         { "type": "text", "text": "yes - per-resource" } ] },
            { "info": { "role": "assistant", "time": { "created": 1789593181200 } },
              "parts": [] }
          ]
        }"#;

        let parsed = opencode(json).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("ses_abc"));
        assert_eq!(parsed.cwd.as_deref(), Some("/repo"));
        // The empty assistant message and the reasoning part are dropped.
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].body, "split the trait?");
        assert_eq!(parsed.turns[1].body, "yes - per-resource");
        assert!(parsed.turns[0].sent_at.is_some());
    }
}
