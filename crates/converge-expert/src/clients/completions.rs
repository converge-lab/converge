//! Client for OpenAI-compatible Chat Completions endpoints.

use async_openai::{
    Client as WireClient,
    config::OpenAIConfig,
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
        ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest, FinishReason,
        ResponseFormat, ResponseFormatJsonSchema,
    },
};
use converge_storage::AgentId;
use thiserror::Error;
use time::OffsetDateTime;

use crate::signals::{self, Meta, Request, Response};

/// Connection and model settings for an OpenAI-compatible provider.
#[derive(Clone)]
pub struct Config {
    /// OpenAI-compatible base URL including the API prefix, for example
    /// http://127.0.0.1:8080/v1.
    pub base_url: String,
    /// Bearer token accepted by the provider. A non-empty placeholder such as
    /// `local` is sufficient when the server has authentication disabled.
    pub api_key: String,
    /// Model name accepted by the server.
    pub model: String,
    /// Context window made available by the provider for this model.
    pub context_window_tokens: u64,
    /// Maximum model output for one signals call. None leaves the limit to
    /// the provider and the model context window.
    pub max_output_tokens: Option<u32>,
    /// Stored Converge identity of the expert model.
    pub producer: AgentId,
}

/// A concrete expert client using the Chat Completions protocol.
pub struct Client {
    wire: WireClient<OpenAIConfig>,
    model: String,
    context_window_tokens: u64,
    max_output_tokens: Option<u32>,
    producer: AgentId,
}

impl Client {
    pub fn new(config: Config) -> Result<Self, Error> {
        let base_url = config.base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return Err(Error::InvalidConfig("base_url is empty".into()));
        }
        if config.api_key.is_empty() {
            return Err(Error::InvalidConfig("api_key is empty".into()));
        }
        let model = config.model.trim().to_owned();
        if model.is_empty() {
            return Err(Error::InvalidConfig("model is empty".into()));
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

        let provider = OpenAIConfig::new()
            .with_api_base(base_url)
            .with_api_key(config.api_key);

        Ok(Self {
            wire: WireClient::with_config(provider),
            model,
            context_window_tokens: config.context_window_tokens,
            max_output_tokens: config.max_output_tokens,
            producer: config.producer,
        })
    }

    /// Discover cross-project signals caused by a new decision.
    pub async fn signals(&self, request: &Request) -> Result<Response, Error> {
        let input = serde_json::to_string(request).map_err(Error::SerializeRequest)?;
        let wire_request = build_request(&self.model, self.max_output_tokens, input);
        let wire_response = self
            .wire
            .chat()
            .create(wire_request)
            .await
            .map_err(Error::Provider)?;

        let usage = wire_response.usage.ok_or(Error::MissingUsage)?;
        let choice = wire_response
            .choices
            .into_iter()
            .next()
            .ok_or(Error::MissingOutput)?;

        match choice.finish_reason {
            Some(FinishReason::Stop) => {}
            Some(FinishReason::Length) => return Err(Error::OutputTruncated),
            Some(reason) => return Err(Error::UnexpectedFinish(format!("{reason:?}"))),
            None => return Err(Error::UnexpectedFinish("missing finish reason".into())),
        }

        if let Some(refusal) = choice.message.refusal.filter(|value| !value.is_empty()) {
            return Err(Error::Refusal(refusal));
        }
        let output = choice.message.content.ok_or(Error::MissingOutput)?;
        let model_response = serde_json::from_str(&output)
            .map_err(|source| Error::InvalidJson { source, output })?;

        Ok(signals::into_response(
            model_response,
            request.decision.decision.id,
            self.producer,
            OffsetDateTime::now_utc(),
            Meta {
                input_tokens: u64::from(usage.prompt_tokens),
                output_tokens: u64::from(usage.completion_tokens),
                context_window_tokens: self.context_window_tokens,
            },
        ))
    }
}

fn build_request(
    model: &str,
    max_output_tokens: Option<u32>,
    input: String,
) -> CreateChatCompletionRequest {
    CreateChatCompletionRequest {
        messages: vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(signals::PROMPT.into()),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(input),
                name: None,
            }),
        ],
        model: model.into(),
        max_completion_tokens: max_output_tokens,
        n: Some(1),
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: ResponseFormatJsonSchema {
                description: Some(
                    "Material effects of the new decision on existing decisions.".into(),
                ),
                name: "signals_response".into(),
                schema: signals::model_response_schema(),
                strict: Some(true),
            },
        }),
        stream: Some(false),
        temperature: Some(0.0),
        ..Default::default()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid Chat Completions client configuration: {0}")]
    InvalidConfig(String),
    #[error("failed to serialize signals request")]
    SerializeRequest(#[source] serde_json::Error),
    #[error("Chat Completions request failed: {0}")]
    Provider(#[source] OpenAIError),
    #[error("Chat Completions response has no token usage")]
    MissingUsage,
    #[error("Chat Completions response has no output text")]
    MissingOutput,
    #[error("Chat Completions provider truncated the structured output")]
    OutputTruncated,
    #[error("Chat Completions provider stopped unexpectedly: {0}")]
    UnexpectedFinish(String),
    #[error("Chat Completions provider refused the request: {0}")]
    Refusal(String),
    #[error("Chat Completions provider returned invalid JSON: {source}; output: {output}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
        output: String,
    },
}
