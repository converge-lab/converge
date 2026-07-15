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

use crate::signals::{self, Request, Response};

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
    /// Maximum model output for one signals call.
    pub max_output_tokens: u32,
    /// Stored Converge identity of the expert model.
    pub producer: AgentId,
}

/// A concrete expert client using the Chat Completions protocol.
pub struct Client {
    wire: WireClient<OpenAIConfig>,
    model: String,
    context_window_tokens: u64,
    max_output_tokens: u32,
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
        if config.max_output_tokens == 0 {
            return Err(Error::InvalidConfig(
                "max_output_tokens must be greater than zero".into(),
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
        signals::validate_request(request).map_err(Error::InvalidRequest)?;
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

        signals::into_response(
            request,
            model_response,
            self.producer,
            OffsetDateTime::now_utc(),
            u64::from(usage.prompt_tokens),
            u64::from(usage.completion_tokens),
            self.context_window_tokens,
        )
        .map_err(Error::InvalidOutput)
    }
}

fn build_request(
    model: &str,
    max_output_tokens: u32,
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
        max_completion_tokens: Some(max_output_tokens),
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
    #[error("invalid signals request: {0}")]
    InvalidRequest(String),
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
    #[error("Chat Completions provider returned an invalid signals response: {0}")]
    InvalidOutput(String),
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use async_openai::types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent, ResponseFormat,
    };
    use converge_storage::{AgentId, DecisionId, Risk, SignalStatus};
    use serde_json::json;
    use time::OffsetDateTime;

    use super::*;
    use crate::signals::{ModelResponse, ModelSignal};

    const TARGET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAY";
    const SOURCE_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAZ";

    fn request() -> Request {
        serde_json::from_value(json!({
            "state": {
                "group": {
                    "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                    "name": "Converge",
                    "description": null,
                    "kind": "shared",
                    "created_at": "2026-01-01T00:00:00Z"
                },
                "projects": [
                    {
                        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAW",
                        "group_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                        "name": "server",
                        "description": null,
                        "created_at": "2026-01-01T00:00:00Z"
                    },
                    {
                        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
                        "group_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                        "name": "mcp",
                        "description": null,
                        "created_at": "2026-01-01T00:00:00Z"
                    }
                ],
                "decisions": [{
                    "id": TARGET_ID,
                    "project_id": "01ARZ3NDEKTSV4RRFFQ69G5FAW",
                    "status": "accepted",
                    "title": "Existing API",
                    "summary": "The tool is called project_suggest.",
                    "context": null,
                    "consequences": null,
                    "alternatives": [],
                    "authors": [],
                    "evidence": [],
                    "captured_at": "2026-01-02T00:00:00Z"
                }],
                "signals": []
            },
            "decision": {
                "id": SOURCE_ID,
                "project_id": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
                "status": "accepted",
                "title": "Rename matching tool",
                "summary": "The only tool is now project_match.",
                "context": null,
                "consequences": null,
                "alternatives": [],
                "authors": [],
                "evidence": [],
                "captured_at": "2026-01-03T00:00:00Z"
            }
        }))
        .expect("valid request")
    }

    #[test]
    fn builds_chat_completion_with_raw_request_and_schema() {
        let request = request();
        let input = serde_json::to_string(&request).unwrap();
        let wire = build_request("qwen-27b", 1_024, input.clone());

        assert_eq!(wire.model, "qwen-27b");
        assert_eq!(wire.max_completion_tokens, Some(1_024));
        assert_eq!(wire.temperature, Some(0.0));
        assert_eq!(wire.messages.len(), 2);

        match &wire.messages[1] {
            ChatCompletionRequestMessage::User(message) => {
                assert_eq!(
                    message.content,
                    ChatCompletionRequestUserMessageContent::Text(input)
                );
            }
            other => panic!("expected user message, got {other:?}"),
        }
        match wire.response_format {
            Some(ResponseFormat::JsonSchema { json_schema }) => {
                assert_eq!(json_schema.name, "signals_response");
                assert_eq!(json_schema.strict, Some(true));
                assert_eq!(json_schema.schema["required"], json!(["signals"]));
            }
            other => panic!("expected JSON Schema response format, got {other:?}"),
        }
    }

    #[test]
    fn completes_domain_signal_fields() {
        let request = request();
        let producer = AgentId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FB0").unwrap();
        let created_at = OffsetDateTime::UNIX_EPOCH;
        let model = ModelResponse {
            signals: vec![ModelSignal {
                targets: vec![DecisionId::from_str(TARGET_ID).unwrap()],
                risk: Risk::WillBreak,
                kind: "tool_contract_change".into(),
                title: "Tool renamed".into(),
                text: "The old tool name no longer exists.".into(),
                consequence: "The MCP flow cannot call the tool.".into(),
                recommendation: "Update the MCP flow to project_match.".into(),
            }],
        };

        let response =
            signals::into_response(&request, model, producer, created_at, 100, 20, 32_768).unwrap();
        let signal = &response.signals[0];

        assert_eq!(signal.source, DecisionId::from_str(SOURCE_ID).unwrap());
        assert_eq!(signal.status, SignalStatus::Proposed);
        assert_eq!(signal.risk, Risk::WillBreak);
        assert_eq!(signal.created_at, created_at);
        assert_eq!(response.meta.input_tokens, 100);
        assert_eq!(response.meta.output_tokens, 20);
        assert_eq!(response.meta.context_window_tokens, 32_768);
    }

    #[test]
    fn rejects_unknown_model_target() {
        let request = request();
        let model = ModelResponse {
            signals: vec![ModelSignal {
                targets: vec![DecisionId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FB1").unwrap()],
                risk: Risk::Coordinate,
                kind: "dependency".into(),
                title: "Unknown target".into(),
                text: "Unknown target.".into(),
                consequence: "Unknown consequence.".into(),
                recommendation: "Unknown recommendation.".into(),
            }],
        };

        let error = signals::into_response(
            &request,
            model,
            AgentId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FB0").unwrap(),
            OffsetDateTime::UNIX_EPOCH,
            1,
            1,
            32_768,
        )
        .unwrap_err();

        assert!(error.contains("unknown decision"));
    }
}
