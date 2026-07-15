#![cfg(feature = "private-fixtures")]

use std::{env, str::FromStr};

use converge_expert::{
    clients::completions::{Client, Config},
    signals::{Request, State},
};
use converge_storage::{AgentId, Decision, DecisionId, Group, Project};
use serde::Deserialize;

const MEMORY_JSON: &str = include_str!("fixtures/signals_memory.json");
const SOURCE_ID: &str = "01KXJQ26CY18KGTRRN76WT32PQ";

#[derive(Deserialize)]
struct Memory {
    schema_version: u32,
    group: Group,
    projects: Vec<Project>,
    decisions: Vec<Decision>,
}

#[tokio::test]
#[ignore = "requires a local llama.cpp server and the private fixtures"]
async fn returns_valid_signals_response_with_live_qwen() {
    let memory: Memory = serde_json::from_str(MEMORY_JSON).expect("valid private memory fixture");
    assert_eq!(memory.schema_version, 1);

    let source_id = DecisionId::from_str(SOURCE_ID).unwrap();
    let source = memory
        .decisions
        .iter()
        .find(|decision| decision.id == source_id)
        .expect("source decision")
        .clone();
    let decisions = memory
        .decisions
        .into_iter()
        .filter(|decision| decision.captured_at < source.captured_at)
        .collect();
    let request = Request {
        state: State {
            group: memory.group,
            projects: memory.projects,
            decisions,
            signals: Vec::new(),
        },
        decision: source,
    };

    let client = Client::new(Config {
        base_url: env::var("LLAMA_CPP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".into()),
        api_key: env::var("LLAMA_CPP_API_KEY").unwrap_or_else(|_| "local".into()),
        model: env::var("LLAMA_CPP_MODEL").unwrap_or_else(|_| "qwen-27b".into()),
        context_window_tokens: 32_768,
        max_output_tokens: 2_400,
        producer: AgentId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FB0").unwrap(),
    })
    .unwrap();

    let response = client.signals(&request).await.unwrap();
    println!("{}", serde_json::to_string_pretty(&response).unwrap());

    assert!(response.meta.input_tokens > 0);
    assert!(response.meta.output_tokens > 0);
    assert_eq!(response.meta.context_window_tokens, 32_768);
    assert!(
        response
            .signals
            .iter()
            .all(|signal| signal.source == source_id)
    );
}
