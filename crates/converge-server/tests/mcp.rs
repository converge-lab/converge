//! The MCP surface over JSON-RPC, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{send, server};
use serde_json::{Value, json};

/// One stateless JSON-RPC round trip against `/mcp`.
async fn rpc(app: &Router, method: &str, params: Value) -> Value {
    let (status, body) = send(
        app,
        "POST",
        "/mcp",
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Call a tool and parse the JSON payload out of its text content.
async fn call(app: &Router, tool: &str, arguments: Value) -> Value {
    let response = rpc(
        app,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await;
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text content: {response}"));
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn tool_round_trip() {
    let (_pg, _store, app) = server().await;

    // The server introduces itself, stateless.
    let init = rpc(
        &app,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0" },
        }),
    )
    .await;
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("project_list"),
        "{init}"
    );

    // The palette is exactly the blessed surface.
    let tools = rpc(&app, "tools/list", json!({})).await;
    let mut names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "decision_add",
            "decision_get",
            "decision_list",
            "message_add",
            "project_bind",
            "project_dismiss",
            "project_list",
            "project_match",
            "project_pick",
            "session_ensure",
        ]
    );

    // Seed a group + project over REST; discover them over MCP.
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "platform", "description": null, "kind": "shared" })),
    )
    .await;
    let group = group["id"].as_str().unwrap().to_owned();
    let (_, project) = send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": group, "name": "converge", "description": null })),
    )
    .await;
    let project = project["id"].as_str().unwrap().to_owned();

    let map = call(&app, "project_list", json!({})).await;
    assert_eq!(map[0]["group_name"], "platform");
    assert_eq!(
        map[0]["projects"][0]["project_id"].as_str().unwrap(),
        project
    );

    // Record a decision, then supersede it with a second one.
    let first = call(
        &app,
        "decision_add",
        json!({
            "project_id": project,
            "title": "Store sessions in redb",
            "summary": "Sessions are append-only; redb fits.",
        }),
    )
    .await["decision_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = call(
        &app,
        "decision_add",
        json!({
            "project_id": project,
            "title": "Store sessions in Postgres",
            "summary": "One backend for everything.",
            "supersedes": [first],
            "alternatives": [{ "option": "Keep redb", "why_rejected": "Second backend to operate" }],
        }),
    )
    .await["decision_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The derived status shows through list and get; edges both ways.
    let superseded = call(&app, "decision_list", json!({ "status": "superseded" })).await;
    assert_eq!(superseded.as_array().unwrap().len(), 1);
    assert_eq!(superseded[0]["decision_id"].as_str().unwrap(), first);
    assert!(
        superseded[0]["captured_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    let got = call(&app, "decision_get", json!({ "decision_id": first })).await;
    assert_eq!(got["decision"]["status"], "superseded");
    assert_eq!(got["edges"]["superseded_by"][0].as_str().unwrap(), second);

    // Authorship was stamped server-side: the deployment user through the
    // generic mcp agent (stateless transport carries no client info), and
    // the agent row was ensured as a side effect.
    let author = &got["decision"]["authors"][0]["user_via_agent"];
    assert!(author["user"].is_string(), "{got}");
    let (_, agents) = send(&app, "GET", "/api/v1/agents", None).await;
    assert_eq!(agents["items"][0]["name"], "mcp");
    assert_eq!(agents["items"][0]["kind"], "tool");
    assert_eq!(agents["items"][0]["id"], author["agent"]);

    // Caller mistakes come back as invalid-params, field named.
    let bad = rpc(
        &app,
        "tools/call",
        json!({ "name": "decision_get", "arguments": { "decision_id": "nonsense" } }),
    )
    .await;
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid decision_id")
            || bad["result"]["isError"].as_bool().unwrap_or(false),
        "{bad}"
    );
}

/// The live-recording loop: ensure this conversation, stream messages,
/// anchor a decision to the exact lines — all over MCP.
#[tokio::test]
async fn ingest_round_trip() {
    let (_pg, _store, app) = server().await;
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "g", "description": null, "kind": "shared" })),
    )
    .await;
    let (_, project) = send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": group["id"], "name": "p", "description": null })),
    )
    .await;
    let project = project["id"].as_str().unwrap();

    // Ensure converges on the natural key, refreshing the title.
    let sid = call(
        &app,
        "session_ensure",
        json!({ "project_id": project, "external": "cc-1", "title": "early" }),
    )
    .await["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let again = call(
        &app,
        "session_ensure",
        json!({ "project_id": project, "external": "cc-1", "title": "final title" }),
    )
    .await;
    assert_eq!(again["session_id"].as_str().unwrap(), sid);

    // Record the exchange; anchor the decision to the second line.
    let ids = call(
        &app,
        "message_add",
        json!({ "session_id": sid, "messages": [
            { "speaker": "maksim", "body": "which way?" },
            { "speaker": "claude", "body": "this way, because…" },
        ]}),
    )
    .await["message_ids"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(ids.len(), 2);
    let anchor = ids[1].as_str().unwrap();

    let decision = call(
        &app,
        "decision_add",
        json!({
            "project_id": project, "title": "Go this way", "summary": "because…",
            "evidence": [anchor],
        }),
    )
    .await["decision_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The anchor rides decision_get, and REST serves the derived excerpt.
    let got = call(&app, "decision_get", json!({ "decision_id": decision })).await;
    assert_eq!(got["decision"]["evidence"][0].as_str().unwrap(), anchor);
    let (_, sources) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{decision}/sources"),
        None,
    )
    .await;
    assert_eq!(sources[0]["session"]["title"], "final title");
    assert_eq!(sources[0]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(sources[0]["anchors"], json!([anchor]));
}

/// The mapping loop's server half: suggest (hint-ranked), bind, dismiss,
/// and the pick fallback contract.
#[tokio::test]
async fn mapping_round_trip() {
    let (_pg, _store, app) = server().await;
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "g", "description": null, "kind": "shared" })),
    )
    .await;
    for name in ["gateway", "billing"] {
        send(
            &app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "group_id": group["id"], "name": name, "description": null })),
        )
        .await;
    }

    // The cwd hint ranks the matching project first.
    let suggested = call(
        &app,
        "project_match",
        json!({ "cwd": "/home/dev/billing", "remote": "git@example.com:corp/billing.git" }),
    )
    .await;
    assert_eq!(suggested["hints"], json!(["billing", "billing"]));
    let candidates = suggested["candidates"].as_array().unwrap();
    assert_eq!(candidates[0]["name"], "billing");
    assert_eq!(candidates[1]["name"], "gateway");

    // Bind existing echoes the payload the marker hook writes.
    let bound = call(
        &app,
        "project_bind",
        json!({ "project_id": candidates[0]["project_id"] }),
    )
    .await;
    assert_eq!(bound["name"], "billing");

    // Create-by-name: the harness has exactly one group, so it is
    // auto-picked without a group_id.
    let created = call(&app, "project_bind", json!({ "name": "fresh" })).await;
    assert!(created["project_id"].is_string());
    assert_eq!(created["name"], "fresh");

    // Dismiss scopes; repo carries the disable flag for the hook.
    let dismissed = call(&app, "project_dismiss", json!({ "scope": "repo" })).await;
    assert_eq!(dismissed["disable"], true);
    let skipped = call(&app, "project_dismiss", json!({ "scope": "session" })).await;
    assert_eq!(skipped["disable"], false);

    // Stateless transport: pick reports the fallback contract.
    let pick = call(&app, "project_pick", json!({})).await;
    assert_eq!(pick["elicitation"], false);
}
