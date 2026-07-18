//! Wire-level checks against stub servers: the resolver really pins the
//! configured origin and credential (no env-var leakage, no model-name
//! inference), replies parse, HTTP failures and the budget surface as
//! typed errors.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use converge_expert::{Endpoint, Error};

/// What the stub saw: headers + body of the last request.
#[derive(Default)]
struct Seen {
    headers: Option<HeaderMap>,
    body: Option<Value>,
}

type Shared = Arc<Mutex<Seen>>;

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn endpoint(value: Value) -> Endpoint {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn openai_compat_round_trip() {
    let seen: Shared = Shared::default();
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(seen): State<Shared>, headers: HeaderMap, body: String| async move {
                    let mut seen = seen.lock().await;
                    seen.headers = Some(headers);
                    seen.body = serde_json::from_str(&body).ok();
                    axum::Json(json!({
                        "id": "cmpl-1",
                        "object": "chat.completion",
                        "created": 0,
                        "model": "qwen3:8b",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "  pong  " },
                            "finish_reason": "stop",
                        }],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 },
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&seen));
    let addr = serve(app).await;

    let client = endpoint(json!({
        "provider": "openai",
        "model": "qwen3:8b",
        "base_url": format!("http://{addr}/v1/"),
        "max_tokens": 64,
    }))
    .client()
    .unwrap();

    let reply = client.reply("be terse", "ping").await.unwrap();
    assert_eq!(reply, "pong", "reply text extracted and trimmed");

    let seen = seen.lock().await;
    let body = seen.body.as_ref().unwrap();
    assert_eq!(body["model"], "qwen3:8b");
    let dump = body.to_string();
    assert!(dump.contains("be terse") && dump.contains("ping"));
}

#[tokio::test]
async fn anthropic_round_trip_carries_the_key() {
    let seen: Shared = Shared::default();
    let app = Router::new()
        .route(
            "/v1/messages",
            post(
                |State(seen): State<Shared>, headers: HeaderMap, body: String| async move {
                    let mut seen = seen.lock().await;
                    seen.headers = Some(headers);
                    seen.body = serde_json::from_str(&body).ok();
                    axum::Json(json!({
                        "id": "msg-1",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-haiku-4-5",
                        "content": [{ "type": "text", "text": "pong" }],
                        "stop_reason": "end_turn",
                        "usage": { "input_tokens": 1, "output_tokens": 1 },
                    }))
                },
            ),
        )
        .with_state(Arc::clone(&seen));
    let addr = serve(app).await;

    let client = endpoint(json!({
        "provider": "anthropic",
        "model": "claude-haiku-4-5",
        "base_url": format!("http://{addr}/v1/"),
        "api_key_cmd": "echo key-from-cmd",
    }))
    .client()
    .unwrap();

    let reply = client.reply("be terse", "ping").await.unwrap();
    assert_eq!(reply, "pong");

    let seen = seen.lock().await;
    let headers = seen.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("x-api-key").map(|v| v.to_str().unwrap()),
        Some("key-from-cmd"),
        "the resolved api_key_cmd secret rides the anthropic key header"
    );
    let body = seen.body.as_ref().unwrap();
    assert_eq!(body["model"], "claude-haiku-4-5");
}

#[tokio::test]
async fn http_failure_is_a_typed_error_not_a_panic() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "model exploded",
            )
        }),
    );
    let addr = serve(app).await;

    let client = endpoint(json!({
        "provider": "openai",
        "model": "m",
        "base_url": format!("http://{addr}/v1/"),
    }))
    .client()
    .unwrap();

    assert!(matches!(client.reply("s", "u").await, Err(Error::Model(_))));
}

#[tokio::test]
async fn the_budget_is_a_hard_bound() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "too late"
        }),
    );
    let addr = serve(app).await;

    let client = endpoint(json!({
        "provider": "openai",
        "model": "m",
        "base_url": format!("http://{addr}/v1/"),
        "timeout_secs": 0.2,
    }))
    .client()
    .unwrap();

    let start = std::time::Instant::now();
    let result = client.reply("s", "u").await;
    assert!(matches!(result, Err(Error::Timeout(_))));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "the call must return at the budget, not at the server's leisure"
    );
}
