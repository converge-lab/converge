//! Evidence over HTTP: session ensure, the append-only stream, and the
//! sources projection (testcontainers — needs Docker).

mod common;

use axum::http::StatusCode;
use common::{send, send_as, server};
use converge_storage::{Memberships, Tokens, Users};
use serde_json::json;

#[tokio::test]
async fn evidence_over_rest() {
    let (_pg, _store, app) = server().await;

    // Group + project to hang everything on.
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
    let project = project["id"].as_str().unwrap().to_owned();

    // Ensure converges: same (kind, external) → same id, title refreshed.
    let new = |title: &str| {
        json!({
            "project_id": project, "kind": "transcript",
            "external": "sess-42", "title": title,
        })
    };
    let (status, first) = send(&app, "POST", "/api/v1/sessions", Some(new("early"))).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (_, again) = send(&app, "POST", "/api/v1/sessions", Some(new("grown"))).await;
    assert_eq!(first["id"], again["id"]);
    let sid = first["id"].as_str().unwrap().to_owned();
    let (status, session) = send(&app, "GET", &format!("/api/v1/sessions/{sid}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["title"], "grown");
    assert!(session["captured_at"].as_str().unwrap().contains('T'));

    // The filterable list.
    let (_, listed) = send(
        &app,
        "GET",
        &format!("/api/v1/sessions?project={project}&kind=transcript"),
        None,
    )
    .await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    // Append twice; the stream reads forward with a cursor.
    let (status, appended) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{sid}/messages"),
        Some(json!([
            { "speaker": "maksim", "body": "should we?" },
            { "speaker": "claude", "body": "yes", "sent_at": "2026-07-01T10:00:00Z" },
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{appended}");
    let ids = appended["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{sid}/messages"),
        Some(json!([{ "speaker": "maksim", "body": "ship it" }])),
    )
    .await;
    let (_, stream) = send(
        &app,
        "GET",
        &format!("/api/v1/sessions/{sid}/messages?limit=2"),
        None,
    )
    .await;
    let items = stream["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["seq"], 0);
    assert_eq!(items[1]["sent_at"], "2026-07-01T10:00:00Z");
    let cursor = stream["next_cursor"].as_str().unwrap();
    let (_, rest) = send(
        &app,
        "GET",
        &format!("/api/v1/sessions/{sid}/messages?cursor={cursor}"),
        None,
    )
    .await;
    let rest = rest["items"].as_array().unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0]["body"], "ship it");

    // A decision anchored to the second message; sources carry the whole
    // three-message window with the anchor marked.
    let anchor = ids[1].as_str().unwrap();
    let (_, decision) = send(
        &app,
        "POST",
        "/api/v1/decisions",
        Some(json!({
            "project_id": project, "status": "accepted", "title": "t", "summary": "",
            "evidence": [anchor],
        })),
    )
    .await;
    let did = decision["id"].as_str().unwrap();
    let (status, sources) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{did}/sources"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sources = sources.as_array().unwrap().clone();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["session"]["id"].as_str().unwrap(), sid);
    assert_eq!(sources[0]["messages"].as_array().unwrap().len(), 3);
    assert_eq!(sources[0]["anchors"], json!([anchor]));

    // Unknown parents are 404, not empty.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    for uri in [
        format!("/api/v1/sessions/{missing}"),
        format!("/api/v1/sessions/{missing}/messages"),
        format!("/api/v1/decisions/{missing}/sources"),
    ] {
        let (status, _) = send(&app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

/// A project that keeps only what its decisions cite: the archive door
/// is shut, the evidence door is not.
#[tokio::test]
async fn archive_off_keeps_only_the_turns_a_decision_cites() {
    let (_pg, store, app) = server().await;
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
    let project = project["id"].as_str().unwrap().to_owned();

    // A project archives until its owner says otherwise.
    let (_, fresh) = send(&app, "GET", &format!("/api/v1/projects/{project}"), None).await;
    assert_eq!(fresh["archive_transcripts"], true, "{fresh}");
    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{project}"),
        Some(json!([{ "set_archive_transcripts": false }])),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // It is the group's call, not one member's: a member who is not
    // the owner cannot decide what everyone else's sessions record.
    let beta = store
        .user_login(converge_storage::Identity {
            provider: "local".into(),
            subject: "beta".into(),
            handle: "beta".into(),
            name: "Beta".into(),
        })
        .await
        .unwrap();
    store
        .token_add(beta, "t".into(), converge_server::auth::hash("cvg_beta"))
        .await
        .unwrap();
    let admin = store.user_lookup("admin").await.unwrap().remove(0).id;
    let gid: converge_storage::GroupId = group["id"].as_str().unwrap().parse().unwrap();
    store
        .member_add(converge_storage::Scope::User(admin), gid, beta)
        .await
        .unwrap();
    let (status, refused) = send_as(
        &app,
        "cvg_beta",
        "PATCH",
        &format!("/api/v1/projects/{project}"),
        Some(json!([{ "set_archive_transcripts": true }])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // The answer to `ensure` carries the policy, so a client learns it
    // in the call it was making anyway.
    let (_, opened) = send(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(json!({
            "project_id": project, "kind": "transcript",
            "external": "quiet-1", "title": "quiet",
        })),
    )
    .await;
    assert_eq!(opened["archive_transcripts"], false, "{opened}");
    assert_eq!(opened["next_ordinal"], 0);
    let session = opened["id"].as_str().unwrap().to_owned();

    // A plain append is refused, and the refusal says what to do.
    let (status, refused) = send(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session}/messages"),
        Some(json!([{ "speaker": "maksim", "body": "a whole conversation" }])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // The turns a decision cites still land, and a decision still has
    // its evidence.
    let (status, decision) = send(
        &app,
        "POST",
        "/mcp",
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
            "name": "decision_add", "arguments": {
                "project_id": project, "title": "t", "summary": "s",
                "conversation": { "external": "quiet-1", "title": "quiet" },
                "evidence_turns": [{ "speaker": "maksim", "body": "this line decided it", "ordinal": 7 }],
            }
        }})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{decision}");
    let recorded: serde_json::Value =
        serde_json::from_str(decision["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(recorded["decision_id"].is_string(), "{decision}");
    let (_, stream) = send(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session}/messages"),
        None,
    )
    .await;
    assert_eq!(stream["items"].as_array().unwrap().len(), 1, "{stream}");

    // And the resume point is the first position nobody holds: the
    // cited turn sits at 7 with nothing below it, so a sender still
    // starts at the beginning rather than skipping the conversation.
    let (_, opened) = send(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(json!({
            "project_id": project, "kind": "transcript",
            "external": "quiet-1", "title": "quiet",
        })),
    )
    .await;
    assert_eq!(opened["next_ordinal"], 0);
}
