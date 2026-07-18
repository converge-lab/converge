//! The Expert — converge's model boundary: one configured actor with one
//! method per operation.
//!
//! Construction fixes the model, the prompts, and the token budget; callers
//! then treat an operation as a plain async function from Request to
//! Response. The comparison subsystem builds its participants through this
//! same constructor (model × prompt), so what it measures is exactly what
//! production runs.

use std::time::Duration;

use converge_storage::AgentId;
use thiserror::Error;
use time::OffsetDateTime;

use crate::clients::genai::{Client, Error as ClientError};
use crate::signals;

pub use crate::clients::genai::Reasoning;

/// Everything an [`Expert`] needs fixed at construction.
#[derive(Debug, Clone)]
pub struct Config {
    /// genai model identifier, adapter namespace included — e.g.
    /// `open_router::qwen/qwen3.5-27b` or `ollama::qwen3`. Credentials and
    /// endpoint resolve from the adapter's environment.
    pub model: String,
    /// System prompt of the signals operation — [`signals::PROMPT`] in
    /// production; the comparison passes variants.
    pub signals_prompt: String,
    /// Sampling temperature. None leaves the provider default — required
    /// for model families that reject the parameter (OpenAI reasoning
    /// models); deterministic setups pass `Some(0.0)`.
    pub temperature: Option<f64>,
    /// Reasoning effort. None leaves the provider default; [`Reasoning::Off`]
    /// explicitly disables thinking where the provider supports an opt-out.
    pub reasoning: Option<Reasoning>,
    /// Ceiling for one whole model call. None — the production default —
    /// leaves calls unbounded: reasoning models legitimately run for
    /// minutes, and only the caller knows its budget.
    pub timeout: Option<Duration>,
    /// Context window made available by the provider for this model.
    pub context_window_tokens: u64,
    /// Maximum model output for one call, reasoning included. None leaves
    /// the limit to the provider and the model context window.
    pub max_output_tokens: Option<u32>,
    /// Stored Converge identity of the expert model.
    pub producer: AgentId,
}

/// A configured expert model.
#[derive(Clone)]
pub struct Expert {
    client: Client,
    signals_prompt: String,
    context_window_tokens: u64,
    producer: AgentId,
}

impl Expert {
    pub fn new(config: Config) -> Result<Self, Error> {
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(Error::InvalidConfig("model is empty".into()));
        }
        if config.signals_prompt.trim().is_empty() {
            return Err(Error::InvalidConfig("signals_prompt is empty".into()));
        }
        if config.context_window_tokens == 0 {
            return Err(Error::InvalidConfig(
                "context_window_tokens must be greater than zero".into(),
            ));
        }
        if config.max_output_tokens == Some(0) {
            return Err(Error::InvalidConfig(
                "max_output_tokens must be greater than zero when set".into(),
            ));
        }
        Ok(Self {
            client: Client::new(
                model,
                config.temperature,
                config.reasoning,
                config.max_output_tokens,
                config.timeout,
            ),
            signals_prompt: config.signals_prompt,
            context_window_tokens: config.context_window_tokens,
            producer: config.producer,
        })
    }

    /// Discover cross-project signals caused by a batch of new decisions.
    pub async fn signals(&self, request: &signals::Request) -> Result<signals::Response, Error> {
        let input = serde_json::to_string(request).map_err(Error::SerializeRequest)?;
        let completion = self
            .client
            .structured(
                &self.signals_prompt,
                input,
                signals::SCHEMA_NAME,
                signals::model_response_schema(),
            )
            .await?;
        let model_response = match serde_json::from_str(&completion.text) {
            Ok(model_response) => model_response,
            Err(source) => {
                return Err(Error::InvalidOutput {
                    source,
                    output: completion.text,
                });
            }
        };
        Ok(signals::into_response(
            model_response,
            self.producer,
            OffsetDateTime::now_utc(),
            signals::Meta {
                input_tokens: completion.input_tokens,
                output_tokens: completion.output_tokens,
                context_window_tokens: self.context_window_tokens,
            },
        ))
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid expert configuration: {0}")]
    InvalidConfig(String),
    #[error("failed to serialize the signals request")]
    SerializeRequest(#[source] serde_json::Error),
    #[error("the model call failed")]
    Client(#[from] ClientError),
    #[error("the model returned invalid JSON; output: {output}")]
    InvalidOutput {
        #[source]
        source: serde_json::Error,
        output: String,
    },
}
