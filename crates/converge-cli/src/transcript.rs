//! Parsing Claude Code JSONL transcripts into evidence turns.
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

/// Parse the whole transcript. Unparseable lines are skipped (a partial
/// trailing line from a concurrent write is simply left for next time).
pub fn read(path: &Path) -> Result<Parsed> {
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

/// A short session title from the first user turn's first line.
pub fn title(turns: &[Turn]) -> String {
    let first = turns
        .iter()
        .find(|t| t.speaker == "user")
        .and_then(|t| t.body.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        return "Claude Code session".into();
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

        let parsed = read(&path).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("s-1"));
        assert_eq!(parsed.cwd.as_deref(), Some("/repo"));
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].speaker, "user");
        assert_eq!(parsed.turns[1].body, "yes — per-resource");
        assert!(parsed.turns[0].sent_at.is_some());
        assert_eq!(title(&parsed.turns), "split the trait?");

        std::fs::remove_file(&path).unwrap();
    }
}
