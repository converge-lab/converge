//! The participant list — `comparison/models/models.toml` parsed into the
//! same [`Config`] production uses, so every measured behavior is a
//! shippable configuration.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use converge_expert::{Config, Reasoning};
use converge_storage::AgentId;
use serde::Deserialize;

/// Stored Converge identity of the comparison expert (the fixture agent).
const PRODUCER: &str = "01J00000000000000000000006";

/// Output ceiling, reasoning included — 8k starved thinking models into
/// `finish_reason: length`; 32k fits every run observed so far.
const MAX_OUTPUT_TOKENS: u32 = 32_000;

/// Per-call ceiling. The archive holds legitimate reasoning runs beyond
/// 300s, so the batch guard sits well above them.
const TIMEOUT: Duration = Duration::from_secs(600);

/// Reported back inside `Meta`; not scored.
const CONTEXT_WINDOW_TOKENS: u64 = 262_144;

#[derive(Debug, Deserialize)]
struct ModelsFile {
    models: Vec<Model>,
}

/// One participant — a model configuration, not just a model id.
#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    /// Display label and the `--models` filter key.
    pub label: String,
    /// OpenRouter model id, without the genai adapter namespace.
    pub id: String,
    /// Grouping for the summary.
    pub tier: String,
    /// Absent = provider default.
    #[serde(default)]
    reasoning: Option<Reasoning>,
    /// Request parameters this model rejects (e.g. "temperature" for the
    /// OpenAI reasoning family).
    #[serde(default)]
    omit_params: Vec<String>,
    /// Upstream pin — kept in the file for the future; not expressible
    /// through genai and therefore ignored.
    #[serde(default)]
    providers: Vec<String>,
}

/// Parse the participant list.
pub fn load(path: &Path) -> anyhow::Result<Vec<Model>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read model list {}", path.display()))?;
    let file: ModelsFile =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    if file.models.iter().any(|model| !model.providers.is_empty()) {
        eprintln!("warning: provider pinning is not expressible through genai and is ignored");
    }
    Ok(file.models)
}

impl Model {
    /// The production expert configuration for this participant, under the
    /// given signals prompt.
    pub fn config(&self, signals_prompt: String) -> Config {
        let temperature = (!self.omit_params.iter().any(|p| p == "temperature")).then_some(0.0);
        Config {
            model: format!("open_router::{}", self.id),
            signals_prompt,
            temperature,
            reasoning: self.reasoning,
            timeout: Some(TIMEOUT),
            context_window_tokens: CONTEXT_WINDOW_TOKENS,
            max_output_tokens: Some(MAX_OUTPUT_TOKENS),
            producer: AgentId::from_str(PRODUCER).expect("the producer id is a valid ULID"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_dir;

    #[test]
    fn real_model_list_parses_into_configs() {
        let models = load(&data_dir().join("models/models.toml")).unwrap();
        assert_eq!(models.len(), 15);

        let flash = models.iter().find(|m| m.label == "qwen3.5-flash").unwrap();
        let config = flash.config("p".into());
        assert_eq!(config.reasoning, Some(Reasoning::Off));
        assert_eq!(config.temperature, Some(0.0));

        let luna = models.iter().find(|m| m.label == "gpt-5.6-luna").unwrap();
        let config = luna.config("p".into());
        assert_eq!(config.temperature, None);
        assert!(config.model.starts_with("open_router::openai/"));
    }
}
