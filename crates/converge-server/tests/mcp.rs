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
            "project_list"
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
