//! The prompt dimension — named system-prompt variants, data-driven.
//!
//! `prompts/prompts.toml` names the variants that run; each entry points at
//! a text file next to it. The production prompt participates as
//! `baseline_v1`, and a test pins that file to
//! [`converge_expert::signals::PROMPT`] so the two cannot drift.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PromptsFile {
    prompts: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    name: String,
    file: String,
}

/// One loaded prompt variant.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Display name and the `--prompts` filter key.
    pub name: String,
    /// The system-prompt text.
    pub text: String,
}

/// Parse the prompt list and read every referenced file.
pub fn load(list: &Path) -> anyhow::Result<Vec<Prompt>> {
    let dir = list
        .parent()
        .context("the prompt list has a parent directory")?;
    let text = std::fs::read_to_string(list).with_context(|| format!("read {}", list.display()))?;
    let file: PromptsFile =
        toml::from_str(&text).with_context(|| format!("parse {}", list.display()))?;
    file.prompts
        .into_iter()
        .map(|entry| {
            let path = dir.join(&entry.file);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read prompt {}", path.display()))?;
            Ok(Prompt {
                name: entry.name,
                text,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_dir;

    fn prompts() -> Vec<Prompt> {
        load(&data_dir().join("prompts/prompts.toml")).unwrap()
    }

    #[test]
    fn prompt_list_parses_and_reads_files() {
        let prompts = prompts();
        assert!(!prompts.is_empty());
        assert!(prompts.iter().all(|p| !p.text.trim().is_empty()));
    }

    /// The declared baseline is the production prompt, byte for byte
    /// (modulo the file's trailing newline) — the whole point of the
    /// comparison is measuring what ships.
    #[test]
    fn baseline_matches_the_production_prompt() {
        let baseline = prompts()
            .into_iter()
            .find(|p| p.name == "baseline_v1")
            .expect("baseline_v1 is declared");
        assert_eq!(
            baseline.text.trim_end(),
            converge_expert::signals::PROMPT.trim_end(),
            "prompts/baseline_v1.md must stay in sync with signals::PROMPT"
        );
    }
}
