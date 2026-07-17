use std::{env, str::FromStr};

use converge_expert::{
    clients::completions::{Client, Config},
    signals::{Request, Response},
};
use converge_storage::{AgentId, DecisionId};

const REQUEST_JSON: &str = include_str!("fixtures/signals_request.json");
const EXPECTED_TARGET_ID: &str = "01J00000000000000000000104";

#[tokio::test]
#[ignore = "requires a local llama.cpp server with Qwen 27B"]
async fn sends_signals_request_and_receives_response() {
    let request: Request =
        serde_json::from_str(REQUEST_JSON).expect("valid signals request fixture");
    let source_id = request.decision.decision.id;
    let expected_target_id = DecisionId::from_str(EXPECTED_TARGET_ID).unwrap();
    let client = Client::new(Config {
        base_url: env::var("LLAMA_CPP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/v1".into()),
        api_key: env::var("LLAMA_CPP_API_KEY").unwrap_or_else(|_| "local".into()),
        model: env::var("LLAMA_CPP_MODEL").unwrap_or_else(|_| "qwen-27b".into()),
        context_window_tokens: 32_768,
        max_output_tokens: None,
        producer: AgentId::from_str("01J00000000000000000000006").unwrap(),
    })
    .unwrap();

    let response: Response = client.signals(&request).await.unwrap();
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
    assert!(
        response
            .signals
            .iter()
            .any(|signal| signal.targets.contains(&expected_target_id)),
        "the response does not contain the expected SSE contract signal"
    );
}
