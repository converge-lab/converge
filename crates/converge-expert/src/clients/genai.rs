//! Client for LLM providers through the genai multi-provider library.
//!
//! One thin seam: a structured chat call — a system prompt, one user
//! payload, and a JSON Schema the provider must enforce — returning the raw
//! output text and token usage. Operation semantics (prompts, request
//! shaping, response conversion) live above, in [`crate::expert`].

use std::time::Duration;

use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec, ReasoningEffort};
use genai::resolver::{AuthData, AuthResolver};
use genai::{ModelIden, WebConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Reasoning effort of a call.
///
/// `Off` is an explicit opt-out for models that think by default; the other
/// tiers map to the provider's effort levels. Distinct from not setting the
/// knob at all, which leaves the provider default untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reasoning {
    Off,
    Low,
    Medium,
    High,
}

impl From<Reasoning> for ReasoningEffort {
    fn from(reasoning: Reasoning) -> Self {
        match reasoning {
            Reasoning::Off => ReasoningEffort::Zero,
            Reasoning::Low => ReasoningEffort::Low,
            Reasoning::Medium => ReasoningEffort::Medium,
            Reasoning::High => ReasoningEffort::High,
        }
    }
}

/// A provider-agnostic structured-chat client.
///
/// The model identifier picks genai's adapter and with it the endpoint and
/// credentials: `open_router::qwen/qwen3.5-27b` (`OPENROUTER_API_KEY`, or genai's `OPEN_ROUTER_API_KEY`),
/// `claude-haiku-4-5` (`ANTHROPIC_API_KEY`), `ollama::qwen3` (local), and so
/// on. Custom endpoints (a local llama.cpp) go through genai's service-target
/// resolution.
#[derive(Clone)]
pub struct Client {
    genai: genai::Client,
    model: String,
    temperature: Option<f64>,
    reasoning: Option<Reasoning>,
    max_output_tokens: Option<u32>,
}

/// The raw outcome of one structured call.
pub struct Completion {
    /// The structured output — the JSON text the schema requested.
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Client {
    /// A client for one model configuration.
    ///
    /// `temperature` is optional because some model families (OpenAI
    /// reasoning models) reject the parameter outright — `None` leaves the
    /// provider default. `timeout` bounds the whole HTTP call; `None` (the
    /// production default) leaves it unbounded — reasoning models
    /// legitimately run for minutes, and only the caller knows its budget.
    pub fn new(
        model: impl Into<String>,
        temperature: Option<f64>,
        reasoning: Option<Reasoning>,
        max_output_tokens: Option<u32>,
        timeout: Option<Duration>,
    ) -> Self {
        // OpenRouter's conventional variable is OPENROUTER_API_KEY; genai's
        // adapter expects OPEN_ROUTER_API_KEY. Accept both — the
        // conventional name wins, and resolving to None falls back to
        // genai's default chain (the adapter's own variable).
        let auth = AuthResolver::from_resolver_fn(
            |model: ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> {
                Ok((model.adapter_kind == AdapterKind::OpenRouter
                    && std::env::var("OPENROUTER_API_KEY").is_ok())
                .then(|| AuthData::from_env("OPENROUTER_API_KEY")))
            },
        );
        let web = WebConfig {
            timeout,
            connect_timeout: Some(CONNECT_TIMEOUT),
            ..Default::default()
        };
        Self {
            genai: genai::Client::builder()
                .with_auth_resolver(auth)
                .with_web_config(web)
                .build(),
            model: model.into(),
            temperature,
            reasoning,
            max_output_tokens,
        }
    }

    /// The configured model identifier.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// One structured chat call bound to `schema`.
    pub async fn structured(
        &self,
        system: &str,
        input: String,
        schema_name: &str,
        schema: Value,
    ) -> Result<Completion, Error> {
        let request = ChatRequest::from_system(system).append_message(ChatMessage::user(input));
        let mut options =
            ChatOptions::default().with_response_format(JsonSpec::new(schema_name, schema));
        if let Some(temperature) = self.temperature {
            options = options.with_temperature(temperature);
        }
        if let Some(reasoning) = self.reasoning {
            options = options.with_reasoning_effort(reasoning.into());
        }
        if let Some(max) = self.max_output_tokens {
            options = options.with_max_tokens(max);
        }
        let response = self
            .genai
            .exec_chat(&self.model, request, Some(&options))
            .await?;
        let input_tokens = tokens(response.usage.prompt_tokens)?;
        let output_tokens = tokens(response.usage.completion_tokens)?;
        let text = response.into_first_text().ok_or(Error::MissingContent)?;
        Ok(Completion {
            text,
            input_tokens,
            output_tokens,
        })
    }
}

fn tokens(count: Option<i32>) -> Result<u64, Error> {
    count
        .and_then(|count| u64::try_from(count).ok())
        .ok_or(Error::MissingUsage)
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("the provider call failed")]
    Provider(#[from] genai::Error),
    #[error("the provider response has no output text")]
    MissingContent,
    #[error("the provider response has no token usage")]
    MissingUsage,
}
