//! Groups + projects over real HTTP semantics, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use axum::http::StatusCode;
use common::{send, server};
use serde_json::{Value, json};

#[tokio::test]
async fn group_crud() {
    let (_pg, _store, app) = server().await;

    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "platform", "description": "owns infra", "kind": "shared" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap().to_owned();

    let (status, group) = send(&app, "GET", &format!("/api/v1/groups/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(group["name"], "platform");
    assert_eq!(group["kind"], "shared");
    // Timestamps are RFC3339 strings, not `time`'s default array encoding.
    assert!(group["created_at"].as_str().unwrap().contains('T'));

    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/groups/{id}"),
        Some(json!([{ "set_name": "platform team" }, { "set_description": null }])),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, edited) = send(&app, "GET", &format!("/api/v1/groups/{id}"), None).await;
    assert_eq!(edited["name"], "platform team");
    assert_eq!(edited["description"], Value::Null);

    let (status, all) = send(&app, "GET", "/api/v1/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["items"].as_array().unwrap().len(), 1);

    // Unknown id → 404 with the error envelope.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, body) = send(&app, "GET", &format!("/api/v1/groups/{missing}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    assert_eq!(body["error"]["message"], "not found");
}

#[tokio::test]
async fn project_crud() {
    let (_pg, _store, app) = server().await;

    let (_, body) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "g", "description": null, "kind": "shared" })),
    )
    .await;
    let group = body["id"].as_str().unwrap().to_owned();

    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": group, "name": "converge", "description": null })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap().to_owned();

    // A second project to exercise the filter.
    let (_, other) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "other", "description": null, "kind": "personal" })),
    )
    .await;
    let other = other["id"].as_str().unwrap().to_owned();
    send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": other, "name": "lab", "description": null })),
    )
    .await;

    let (status, listed) = send(
        &app,
        "GET",
        &format!("/api/v1/projects?group={group}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = listed["items"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"].as_str().unwrap(), id);

    let (_, all) = send(&app, "GET", "/api/v1/projects", None).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    let (_, limited) = send(&app, "GET", "/api/v1/projects?limit=1", None).await;
    assert_eq!(limited["items"].as_array().unwrap().len(), 1);
    assert!(limited["next_cursor"].is_string());

    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/projects/{id}"),
        Some(json!([{ "set_description": "the memory server" }])),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, edited) = send(&app, "GET", &format!("/api/v1/projects/{id}"), None).await;
    assert_eq!(edited["description"], "the memory server");

    // A project pointing at a missing group is the caller's error: 400.
    let missing = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": missing, "name": "orphan", "description": null })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing referenced record")
    );
}

#[tokio::test]
async fn pagination() {
    let (_pg, _store, app) = server().await;
    for name in ["a", "b", "c"] {
        send(
            &app,
            "POST",
            "/api/v1/groups",
            Some(json!({ "name": name, "description": null, "kind": "shared" })),
        )
        .await;
    }

    // Full page: cursor points at its last item.
    let (status, first) = send(&app, "GET", "/api/v1/groups?limit=2", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    assert_eq!(first["items"][1]["id"].as_str().unwrap(), cursor);

    // Following the cursor: the remainder, exhausted.
    let (_, rest) = send(
        &app,
        "GET",
        &format!("/api/v1/groups?limit=2&cursor={cursor}"),
        None,
    )
    .await;
    let items = rest["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "a");
    assert!(rest["next_cursor"].is_null());

    // No pages overlap and nothing is lost.
    assert_ne!(items[0]["id"], first["items"][0]["id"]);
    assert_ne!(items[0]["id"], first["items"][1]["id"]);

    // Unpaginated read: everything, no cursor.
    let (_, all) = send(&app, "GET", "/api/v1/groups", None).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 3);
    assert!(all["next_cursor"].is_null());
}

#[tokio::test]
async fn users_me() {
    let (_pg, _store, app) = server().await;

    // `me` resolves to the configured single-user identity, created on
    // first read.
    let (status, me) = send(&app, "GET", "/api/v1/users/me", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["handle"], "admin");
    assert_eq!(me["name"], "Admin");

    // Deterministic: the second read is the same user.
    let (_, again) = send(&app, "GET", "/api/v1/users/me", None).await;
    assert_eq!(again["id"], me["id"]);
}
