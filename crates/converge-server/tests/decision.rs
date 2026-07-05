//! Decisions over HTTP: CRUD, edit batches, filters, and the graph
//! projection (testcontainers — needs Docker).

mod common;

use axum::Router;
use axum::http::StatusCode;
use common::{send, server};
use converge_storage::{AgentKind, Agents, NewAgent, NewUser, Users};
use serde_json::{Value, json};

/// Group + project over the API; returns their ids.
async fn seed(app: &Router) -> (String, String) {
    let (_, group) = send(
        app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "g", "description": null, "kind": "shared" })),
    )
    .await;
    let group = group["id"].as_str().unwrap().to_owned();
    let (_, project) = send(
        app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": group, "name": "p", "description": null })),
    )
    .await;
    (group, project["id"].as_str().unwrap().to_owned())
}

/// POST a decision built from `body`; returns its id.
async fn add(app: &Router, body: Value) -> String {
    let (status, body) = send(app, "POST", "/api/v1/decisions", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn decision_crud() {
    let (_pg, store, app) = server().await;
    let (group, project) = seed(&app).await;

    // Minimal create — the collections default to empty on the wire.
    let id = add(
        &app,
        json!({ "project_id": project, "status": "proposed", "title": "ADR-1", "summary": "s" }),
    )
    .await;
    let (status, decision) = send(&app, "GET", &format!("/api/v1/decisions/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decision["title"], "ADR-1");
    assert_eq!(decision["status"], "proposed");
    assert_eq!(decision["alternatives"], json!([]));
    assert_eq!(decision["authors"], json!([]));
    assert!(decision["captured_at"].as_str().unwrap().contains('T'));

    // Authors and alternatives pass through. Users/agents have no REST
    // surface yet (that's the auth slice) — seed them via the store.
    let user = store
        .user_ensure(NewUser {
            handle: "singulared".into(),
            name: "Maksim".into(),
        })
        .await
        .unwrap();
    let agent = store
        .agent_ensure(NewAgent {
            kind: AgentKind::Model,
            name: "claude".into(),
        })
        .await
        .unwrap();
    let authored = add(
        &app,
        json!({
            "project_id": project, "status": "accepted", "title": "ADR-2", "summary": "s",
            "authors": [{ "user": user }, { "user_via_agent": { "user": user, "agent": agent } }],
            "alternatives": [{ "option": "other", "why_rejected": "slower" }],
        }),
    )
    .await;
    let (_, decision) = send(&app, "GET", &format!("/api/v1/decisions/{authored}"), None).await;
    assert_eq!(decision["authors"].as_array().unwrap().len(), 2);
    assert_eq!(decision["alternatives"][0]["why_rejected"], "slower");

    // The atomic edit batch.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/decisions/{id}"),
        Some(json!([{ "set_status": "accepted" }, { "set_context": "ctx" }])),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, edited) = send(&app, "GET", &format!("/api/v1/decisions/{id}"), None).await;
    assert_eq!(edited["status"], "accepted");
    assert_eq!(edited["context"], "ctx");

    // Filters compose; status matches the derived status.
    let (_, by_project) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions?project={project}"),
        None,
    )
    .await;
    assert_eq!(by_project["items"].as_array().unwrap().len(), 2);
    let (_, by_group) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions?group={group}&status=accepted&limit=1"),
        None,
    )
    .await;
    assert_eq!(by_group["items"].as_array().unwrap().len(), 1);

    // `superseded` is derived — storing it is the caller's error.
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/decisions",
        Some(json!({ "project_id": project, "status": "superseded", "title": "x", "summary": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("derived")
    );

    // Unknown ids → 404 on every read/write path.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    for (method, uri, body) in [
        ("GET", format!("/api/v1/decisions/{missing}"), None),
        (
            "PATCH",
            format!("/api/v1/decisions/{missing}"),
            Some(json!([{ "set_title": "x" }])),
        ),
    ] {
        let (status, _) = send(&app, method, &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn relation_projections() {
    let (_pg, _store, app) = server().await;
    let (group, project) = seed(&app).await;
    let id = add(
        &app,
        json!({ "project_id": project, "status": "accepted", "title": "A", "summary": "" }),
    )
    .await;

    // The nested reads mirror the canonical flat filters…
    let (status, projects) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{group}/projects"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects["items"].as_array().unwrap().len(), 1);
    assert_eq!(projects["items"][0]["id"].as_str().unwrap(), project);
    let (status, decisions) = send(
        &app,
        "GET",
        &format!("/api/v1/projects/{project}/decisions?status=accepted"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decisions["items"][0]["id"].as_str().unwrap(), id);

    // …but the bound parent must exist: 404, not an empty list.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{missing}/projects"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/projects/{missing}/decisions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Re-binding the path-bound parent via query is rejected, not ignored.
    let (status, body) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{group}/projects?group={group}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bound by the path")
    );
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/projects/{project}/decisions?project={project}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The group feed spans the group's projects; `?project=` narrows within
    // it (child axis — allowed), `?group=` re-binds (rejected).
    let (status, feed) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{group}/decisions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(feed["items"][0]["id"].as_str().unwrap(), id);
    let (status, narrowed) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{group}/decisions?project={project}&status=accepted"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(narrowed["items"].as_array().unwrap().len(), 1);
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{group}/decisions?group={group}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/groups/{missing}/decisions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn decision_graph() {
    let (_pg, _store, app) = server().await;
    let (_, project) = seed(&app).await;

    let a = add(
        &app,
        json!({ "project_id": project, "status": "accepted", "title": "A", "summary": "" }),
    )
    .await;
    let b = add(
        &app,
        json!({
            "project_id": project, "status": "accepted", "title": "B", "summary": "",
            "supersedes": [a],
        }),
    )
    .await;

    // A reads as superseded — derived from the inbound edge, not stored.
    let (_, got) = send(&app, "GET", &format!("/api/v1/decisions/{a}"), None).await;
    assert_eq!(got["status"], "superseded");
    let (_, listed) = send(&app, "GET", "/api/v1/decisions?status=superseded", None).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    assert_eq!(listed["items"][0]["id"].as_str().unwrap(), a);

    // The projection carries both directions.
    let (status, edges) = send(&app, "GET", &format!("/api/v1/decisions/{b}/edges"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(edges["supersedes"], json!([a]));
    let (_, edges) = send(&app, "GET", &format!("/api/v1/decisions/{a}/edges"), None).await;
    assert_eq!(edges["superseded_by"], json!([b]));

    // Removing the last inbound edge restores the stored status.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/decisions/{b}"),
        Some(json!([{ "remove_supersedes": a }])),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, restored) = send(&app, "GET", &format!("/api/v1/decisions/{a}"), None).await;
    assert_eq!(restored["status"], "accepted");

    // Cross-refs round-trip with `why`, visible from both ends.
    send(
        &app,
        "PATCH",
        &format!("/api/v1/decisions/{b}"),
        Some(json!([{ "add_related": { "to": a, "why": "shared context" } }])),
    )
    .await;
    let (_, edges) = send(&app, "GET", &format!("/api/v1/decisions/{b}/edges"), None).await;
    assert_eq!(
        edges["related_to"],
        json!([{ "id": a, "why": "shared context" }])
    );
    let (_, edges) = send(&app, "GET", &format!("/api/v1/decisions/{a}/edges"), None).await;
    assert_eq!(
        edges["related_by"],
        json!([{ "id": b, "why": "shared context" }])
    );

    // Self-loops are the caller's error.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/decisions/{b}"),
        Some(json!([{ "add_supersedes": b }])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Unknown decision → 404 on the projection too.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{missing}/edges"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
