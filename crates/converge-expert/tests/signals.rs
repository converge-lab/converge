//! The signals job end to end: the golden fixture parses, the wire
//! carries the schema constraint, validation narrows the reply — and an
//! ignored live test runs the whole judgment against a real model.
//!
//! The fixture is laxkey's Workboard scenario (PR #4), reshaped to the
//! retrieval-fed contract: a web-app decision to cache full SSE payloads
//! colliding with the server's accepted "ids and revisions only" SSE
//! contract (decision …104) — the expected conflict.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use converge_expert::signals::{Request, discover};
use converge_expert::{Client, Endpoint};
use serde_json::{Value, json};
use tokio::sync::Mutex;

const FIXTURE: &str = include_str!("fixtures/signals.json");
const EXPECTED_TARGET: &str = "01J00000000000000000000104";

fn fixture() -> Request {
    serde_json::from_str(FIXTURE).expect("the golden fixture matches the contract")
}

#[test]
fn the_fixture_parses_and_names_the_players() {
    let request = fixture();
    assert_eq!(request.decision.project, "web-app");
    assert_eq!(request.candidates.len(), 25);
    assert!(!request.signals.is_empty());
    assert!(
        request
            .candidates
            .iter()
            .any(|c| c.decision.id.to_string() == EXPECTED_TARGET),
        "the expected conflict target is among the candidates"
    );
}

async fn stub(reply: Value) -> (SocketAddr, Arc<Mutex<Option<Value>>>) {
    let seen: Arc<Mutex<Option<Value>>> = Arc::default();
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(seen): State<Arc<Mutex<Option<Value>>>>, body: String| async move {
                    *seen.lock().await = serde_json::from_str(&body).ok();
                    axum::Json(json!({
                        "id": "cmpl-1",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "stub",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": reply.to_string() },
                            "finish_reason": "stop",
                        }],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 },
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, seen)
}

fn client(addr: SocketAddr) -> Client {
    let endpoint: Endpoint = serde_json::from_value(json!({
        "provider": "openai",
        "model": "stub",
        "base_url": format!("http://{addr}/v1/"),
    }))
    .unwrap();
    endpoint.client().unwrap()
}

#[tokio::test]
async fn the_job_constrains_the_wire_and_validates_the_reply() {
    let request = fixture();
    let bogus = converge_storage::DecisionId::new().to_string();
    let (addr, seen) = stub(json!({
        "signals": [{
            "targets": [EXPECTED_TARGET, bogus],
            "kind": "Contract Divergence",
            "tier": "conflict",
            "title": "full payloads contradict the SSE contract",
            "text": "the cache update consumes fields SSE no longer carries",
            "consequence": "boards render stale data",
            "recommendation": "consume ids and refetch",
        }],
    }))
    .await;

    let drafts = discover(&client(addr), &request).await.unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].targets.len(), 1, "the bogus target is dropped");
    assert_eq!(drafts[0].targets[0].to_string(), EXPECTED_TARGET);
    assert_eq!(drafts[0].kind, "contract_divergence");

    // The request rode structured output: temperature pinned to zero and
    // the schema on the wire, with the fixture JSON as the user turn.
    let seen = seen.lock().await.clone().unwrap();
    assert_eq!(seen["temperature"], 0.0);
    assert_eq!(
        seen["response_format"]["type"], "json_schema",
        "{}",
        seen["response_format"]
    );
    let schema = &seen["response_format"]["json_schema"]["schema"];
    assert_eq!(schema["properties"]["signals"]["type"], "array");
    let user = seen["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains(EXPECTED_TARGET) && user.contains("web-app"));
}

#[tokio::test]
async fn an_empty_judgment_is_a_success() {
    let (addr, _) = stub(json!({ "signals": [] })).await;
    let drafts = discover(&client(addr), &fixture()).await.unwrap();
    assert!(drafts.is_empty());
}

/// The real judgment, against whatever `[expert]`-shaped endpoint the
/// environment names — off by default (needs a running model).
///
/// ```sh
/// CONVERGE_EXPERT_URL=http://127.0.0.1:11434/v1/ \
/// CONVERGE_EXPERT_MODEL=qwen3:8b \
/// cargo test -p converge-expert --test signals -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "needs a live model endpoint (CONVERGE_EXPERT_URL / CONVERGE_EXPERT_MODEL)"]
async fn live_model_finds_the_sse_conflict() {
    let url = std::env::var("CONVERGE_EXPERT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:11434/v1/".into());
    let model = std::env::var("CONVERGE_EXPERT_MODEL").unwrap_or_else(|_| "qwen3:8b".into());
    let endpoint: Endpoint = serde_json::from_value(json!({
        "provider": "openai",
        "model": model,
        "base_url": url,
        "timeout_secs": 600.0,
        "max_tokens": 8192,
    }))
    .unwrap();
    // Feed the job what production feeds it: the retrieval-ranked top-K,
    // not the whole corpus (a crude lexical rank stands in for the
    // server's tsvector search here).
    let mut request = fixture();
    let subject = format!(
        "{} {} {}",
        request.decision.decision.title,
        request.decision.decision.summary,
        request.decision.decision.context.as_deref().unwrap_or("")
    )
    .to_lowercase();
    // Content words only — a stopword-scored rank buries the real hits
    // (postgres's english config does this for the production path).
    const STOP: &[&str] = &[
        "the", "and", "with", "that", "this", "from", "into", "over", "only", "not", "are", "for",
        "its", "has", "have", "when", "then", "than", "them", "each", "every",
    ];
    let words: Vec<&str> = subject
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOP.contains(w))
        .collect();
    request.candidates.sort_by_key(|c| {
        let hay = format!("{} {}", c.decision.title, c.decision.summary).to_lowercase();
        std::cmp::Reverse(words.iter().filter(|w| hay.contains(**w)).count())
    });
    request.candidates.truncate(8);
    let drafts = discover(&endpoint.client().unwrap(), &request)
        .await
        .unwrap();

    println!("{}", serde_json::to_string_pretty(&drafts).unwrap());
    assert!(
        drafts
            .iter()
            .any(|d| d.targets.iter().any(|t| t.to_string() == EXPECTED_TARGET)),
        "no draft targets the SSE contract decision"
    );
}
