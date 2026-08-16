//! The detection pass end to end against a stub model endpoint: seed two
//! projects over REST, run the pass, watch the draft become a stored
//! signal with expert authorship — and the re-raise ban make the pass
//! idempotent (testcontainers — needs Docker).

mod common;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use common::{send, server};
use converge_expert::{Config, Registry};
use converge_server::Expert;
use converge_storage::{AgentKind, Agents, NewAgent};
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// A chat-completions stub that answers with `reply` and records the
/// request bodies it saw.
async fn stub(reply: Value) -> (std::net::SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(seen): State<Arc<Mutex<Vec<Value>>>>, body: String| async move {
                    seen.lock().await.push(serde_json::from_str(&body).unwrap());
                    axum::Json(json!({
                        "id": "cmpl-1", "object": "chat.completion", "created": 0,
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

fn registry(addr: std::net::SocketAddr) -> Registry {
    let config: Config = serde_json::from_value(json!({
        "models": { "stub": {
            "provider": "openai", "model": "stub",
            "base_url": format!("http://{addr}/v1/"),
        }},
        "jobs": { "signals": "stub" },
    }))
    .unwrap();
    Registry::new(&config).unwrap()
}

#[tokio::test]
async fn detection_writes_stamped_signals_once() {
    let (_pg, store, app) = server().await;

    // Two projects; the subject in one, the expected target in the other,
    // plus a same-project decoy retrieval must filter out.
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "team", "kind": "shared" })),
    )
    .await;
    async fn project(app: &Router, group: &Value, name: &str) -> String {
        let (_, p) = send(
            app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "group_id": group["id"], "name": name })),
        )
        .await;
        p["id"].as_str().unwrap().to_string()
    }
    async fn decision(app: &Router, project: &str, title: &str) -> String {
        let (_, d) = send(
            app,
            "POST",
            "/api/v1/decisions",
            Some(json!({
                "project_id": project, "status": "accepted",
                "title": title, "summary": "",
                "context": null, "consequences": null,
            })),
        )
        .await;
        d["id"].as_str().unwrap().to_string()
    }
    let web = project(&app, &group, "web-app").await;
    let srv = project(&app, &group, "server").await;
    let target = decision(&app, &srv, "Send only ids and revisions over SSE").await;
    let decoy = decision(&app, &web, "Cache SSE events in memory").await;
    let subject = decision(&app, &web, "Update the cache with full SSE payloads").await;

    let (addr, seen) = stub(json!({
        "signals": [{
            "targets": [target],
            "kind": "contract_divergence",
            "tier": "conflict",
            "title": "SSE payload contract divergence",
            "text": "full payloads contradict the ids-only contract",
            "consequence": "the cache strategy breaks",
            "recommendation": "align the SSE payload",
        }],
    }))
    .await;
    let agent = store
        .agent_ensure(NewAgent {
            kind: AgentKind::Model,
            name: "expert".into(),
        })
        .await
        .unwrap();
    let expert = Expert::new(store.clone(), registry(addr), agent);

    // The pass: one draft becomes one stored signal, expert-stamped.
    let written = expert.run(subject.parse().unwrap()).await.unwrap();
    assert_eq!(written, 1);
    let (_, page) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{target}/signals"),
        None,
    )
    .await;
    let signal = &page["items"][0];
    assert_eq!(signal["source"], json!(subject));
    assert_eq!(signal["targets"], json!([target]));
    assert_eq!(signal["tier"], "conflict");
    assert_eq!(signal["status"], "proposed");
    assert_eq!(signal["produced_by"]["agent"], json!(agent.to_string()));

    // What the model saw: candidates from the whole group, the
    // subject's own project included — several people share a project,
    // so a same-project decision is as real a collision target as a
    // cross-project one. Only the subject itself is excluded.
    {
        let seen = seen.lock().await;
        assert_eq!(seen.len(), 1);
        let user = seen[0]["messages"][1]["content"].as_str().unwrap();
        let request: Value = serde_json::from_str(user).unwrap();
        let candidates = request["candidates"].as_array().unwrap();
        assert!(!candidates.is_empty());
        assert!(
            candidates
                .iter()
                .any(|c| c["id"] == json!(decoy.to_string())),
            "the same-project decoy {decoy} belongs in the candidates: {candidates:?}"
        );
        assert!(
            candidates
                .iter()
                .all(|c| c["id"] != json!(subject.to_string())),
            "the subject itself must not be a candidate"
        );
        // Cache layout: the volatile subject serializes last *on the
        // wire* (the parsed Value alphabetizes keys — check the string).
        let subject_at = user.find("\"decision\":").unwrap();
        assert!(user.find("\"candidates\":").unwrap() < subject_at);
        assert!(user.find("\"signals\":").unwrap() < subject_at);
    }

    // Idempotent: the re-raise ban absorbs the duplicate draft.
    let written = expert.run(subject.parse().unwrap()).await.unwrap();
    assert_eq!(written, 0);
    let (_, page) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{target}/signals"),
        None,
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);

    // No job binding: the pass is a no-op that never calls the model.
    let inert = Expert::new(store.clone(), Registry::default(), agent);
    assert_eq!(inert.run(subject.parse().unwrap()).await.unwrap(), 0);
    assert_eq!(seen.lock().await.len(), 2);

    // The MCP loop closes the delivery: the agent lists the proposed
    // signal and resolves it with the user's verdict.
    let mcp = |tool: &str, arguments: Value| {
        let app = app.clone();
        let (tool, arguments) = (tool.to_string(), arguments);
        async move {
            let (_, body) = send(
                &app,
                "POST",
                "/mcp",
                Some(json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": tool, "arguments": arguments },
                })),
            )
            .await;
            let text = body["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("no text content: {body}"));
            serde_json::from_str::<Value>(text).unwrap()
        }
    };
    let listed = mcp(
        "signal_list",
        json!({ "decision_id": target, "status": "proposed" }),
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["tier"], "conflict");
    let sid = listed[0]["signal_id"].as_str().unwrap().to_string();

    let resolved = mcp(
        "signal_resolve",
        json!({ "signal_id": sid, "status": "confirmed" }),
    )
    .await;
    assert_eq!(resolved["status"], "confirmed");
    let (_, got) = send(&app, "GET", &format!("/api/v1/signals/{sid}"), None).await;
    assert_eq!(got["status"], "confirmed");
    assert!(
        got["resolved_by"]["user_via_agent"]["user"].is_string(),
        "the verdict is stamped as the user through the calling agent: {got}"
    );

    // Backfill sweeps the whole corpus through the same pass. The stub
    // proposes the same target for every subject, so: subject's pair is
    // already observed (re-raise ban absorbs it), target-as-subject
    // self-targets (storage rejects the draft, non-fatally), and
    // decoy-as-subject is a genuinely new pair — exactly one write.
    let stats = expert
        .backfill(converge_storage::DecisionFilter::default())
        .await
        .unwrap();
    assert_eq!(stats.examined, 3, "target, decoy, subject");
    assert_eq!(stats.written, 1, "only the decoy→target pair is new");
    assert_eq!(
        stats.failed, 0,
        "rejected drafts are absorbed, not failures"
    );
    // And a second sweep is fully absorbed — idempotence.
    let stats = expert
        .backfill(converge_storage::DecisionFilter::default())
        .await
        .unwrap();
    assert_eq!(stats.written, 0);
    // Without a job binding backfill is a no-op that examines nothing.
    let stats = inert
        .backfill(converge_storage::DecisionFilter::default())
        .await
        .unwrap();
    assert_eq!(stats, converge_server::Backfill::default());
}

/// The ask surface's guardrails (the streaming path itself needs a live
/// model): no configured `ask` job answers 503; an invisible group 404s.
#[tokio::test]
async fn ask_guardrails() {
    let (_pg, store, app) = server().await;
    let group = {
        use converge_storage::{Groups, NewGroup, Users};
        let admin = store.user_lookup("admin").await.unwrap().remove(0).id;
        store
            .group_add(
                admin,
                NewGroup {
                    name: "asked".into(),
                    description: None,
                    kind: converge_storage::GroupKind::Shared,
                },
            )
            .await
            .unwrap()
    };
    // The harness registry has no jobs: asking is refused loudly, not
    // hung — the fail-open rule is for enrichment, not for answers.
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/expert/ask",
        Some(json!({ "group_id": group, "question": "what do we know?" })),
    )
    .await;
    assert_eq!(status.as_u16(), 503, "{body}");

    let ghost = converge_storage::GroupId::new();
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/expert/ask",
        Some(json!({ "group_id": ghost, "question": "hello?" })),
    )
    .await;
    assert_eq!(status.as_u16(), 404);
}
