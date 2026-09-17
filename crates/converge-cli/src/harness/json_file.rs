//! JSON files other tools own, written without reformatting them.
//!
//! serde's pretty printer is two spaces and a trailing newline. The
//! files we edit — opencode's `opencode.json`, Claude's `settings.json`,
//! Cursor's `hooks.json` — are however their owners left them, and a
//! registration that rewrites five hundred lines to change three is a
//! diff nobody asked for. So the indent unit and the trailing-newline
//! state are copied from the text that was there.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use serde_json::ser::PrettyFormatter;

/// Read a JSON file, keeping the raw text so [`write()`] can match it. A
/// missing file reads as `default` with empty text.
pub fn read(path: &Path, default: Value) -> Result<(Value, String)> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let value = serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid JSON", path.display()))?;
            Ok((value, text))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((default, String::new())),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Write `doc` formatted like `like`: the first indented line's leading
/// whitespace is the indent unit, and the file ends with a newline only
/// if `like` did. Nothing to copy from (a new file) means two spaces and
/// a newline.
pub fn write(path: &Path, doc: &Value, like: &str) -> Result<()> {
    let indent = like
        .lines()
        .find(|line| line.starts_with([' ', '\t']) && !line.trim().is_empty())
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .unwrap_or("  ");
    let newline = like.is_empty() || like.ends_with('\n');

    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(
        &mut out,
        PrettyFormatter::with_indent(indent.as_bytes()),
    );
    doc.serialize(&mut ser)?;
    if newline {
        out.push(b'\n');
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(path, out).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_the_owners_indent_and_newline_state() {
        let dir = std::env::temp_dir().join(format!("cvg-json-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.json");

        // Four spaces, no trailing newline: the shape opencode.json has.
        std::fs::write(&path, "{\n    \"a\": {\n        \"b\": 1\n    }\n}").unwrap();
        let (mut doc, like) = read(&path, json!({})).unwrap();
        doc["c"] = json!(2);
        write(&path, &doc, &like).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n    \"a\": {\n        \"b\": 1\n    },\n    \"c\": 2\n}"
        );

        // Tabs, with a newline.
        std::fs::write(&path, "{\n\t\"a\": 1\n}\n").unwrap();
        let (doc, like) = read(&path, json!({})).unwrap();
        write(&path, &doc, &like).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n\t\"a\": 1\n}\n"
        );

        // A new file: serde's defaults.
        let fresh = dir.join("new.json");
        let (doc, like) = read(&fresh, json!({ "x": [] })).unwrap();
        assert!(like.is_empty());
        write(&fresh, &doc, &like).unwrap();
        assert_eq!(
            std::fs::read_to_string(&fresh).unwrap(),
            "{\n  \"x\": []\n}\n"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
