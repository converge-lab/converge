//! The ACL end to end: two users over REST — invisible groups are
//! 404-shaped everywhere, membership grants exactly the group, the owner
//! alone manages it, leaving revokes (testcontainers — needs Docker).

mod common;

use axum::http::StatusCode;
use common::{send, send_as, server};
use converge_server::auth;
use converge_storage::{Identity, Tokens, Users};
use serde_json::json;

const BOB: &str = "cvg_bob";

#[tokio::test]
async fn membership_is_visibility() {
    let (_pg, store, app) = server().await;
    // A second authenticated user, provisioned like the bootstrap flow.
    let bob = store
        .user_login(Identity {
            provider: "github".into(),
            subject: "17".into(),
            handle: "bob".into(),
            name: "Bob".into(),
        })
        .await
        .unwrap();
    store
        .token_add(bob, "bob".into(), auth::hash(BOB))
        .await
        .unwrap();

    // The admin's world: a group, a project, a decision, a session.
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "platform", "kind": "shared" })),
    )
    .await;
    let gid = group["id"].as_str().unwrap().to_string();
    let (_, project) = send(
        &app,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": gid, "name": "api" })),
    )
    .await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, decision) = send(
        &app,
        "POST",
        "/api/v1/decisions",
        Some(json!({
            "project_id": pid, "status": "accepted",
            "title": "Rate limits are per token", "summary": "",
            "context": null, "consequences": null,
        })),
    )
    .await;
    let did = decision["id"].as_str().unwrap().to_string();

    // The group is created owned by its creator.
    let (_, me) = send(&app, "GET", "/api/v1/users/me", None).await;
    assert_eq!(group_owner(&app, &gid).await, me["id"]);

    // Bob sees none of it: lists are empty, gets are 404, search finds
    // nothing, the relation projections 404 on the bound parent.
    let (_, groups) = send_as(&app, BOB, "GET", "/api/v1/groups", None).await;
    assert_eq!(groups["items"], json!([]));
    let (_, projects) = send_as(&app, BOB, "GET", "/api/v1/projects", None).await;
    assert_eq!(projects["items"], json!([]));
    let (status, _) = send_as(&app, BOB, "GET", &format!("/api/v1/groups/{gid}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_as(&app, BOB, "GET", &format!("/api/v1/decisions/{did}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_as(
        &app,
        BOB,
        "GET",
        &format!("/api/v1/groups/{gid}/decisions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, found) = send_as(&app, BOB, "GET", "/api/v1/decisions?q=rate+limits", None).await;
    assert_eq!(found["items"], json!([]));
    // Bob can't write into the invisible group either.
    let (status, _) = send_as(
        &app,
        BOB,
        "POST",
        "/api/v1/projects",
        Some(json!({ "group_id": gid, "name": "sneak" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The user directory shows Bob only himself.
    let (_, users) = send_as(&app, BOB, "GET", "/api/v1/users", None).await;
    let handles: Vec<_> = users["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["handle"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(handles, vec!["bob"]);

    // Membership management is the owner's: Bob can't self-invite, and
    // an unknown handle is a clear error.
    let (status, _) = send_as(
        &app,
        BOB,
        "POST",
        &format!("/api/v1/groups/{gid}/members"),
        Some(json!({ "handle": "bob" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = send(
        &app,
        "POST",
        &format!("/api/v1/groups/{gid}/members"),
        Some(json!({ "handle": "nobody" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // The owner adds Bob by handle; the world opens exactly this wide.
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/groups/{gid}/members"),
        Some(json!({ "handle": "bob" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Ownership contains membership: the roster leads with the owner,
    // marked, then the invited members.
    let (_, members) = send(&app, "GET", &format!("/api/v1/groups/{gid}/members"), None).await;
    assert_eq!(members[0]["handle"], "admin");
    assert_eq!(members[0]["owner"], true);
    assert_eq!(members[1]["handle"], "bob");
    assert_eq!(members[1]["owner"], false);
    let (status, got) = send_as(&app, BOB, "GET", &format!("/api/v1/decisions/{did}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["title"], "Rate limits are per token");
    let (_, found) = send_as(&app, BOB, "GET", "/api/v1/decisions?q=rate+limits", None).await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    // Members write: Bob records a decision in the shared project.
    let (status, _) = send_as(
        &app,
        BOB,
        "POST",
        "/api/v1/decisions",
        Some(json!({
            "project_id": pid, "status": "accepted",
            "title": "Bob was here", "summary": "",
            "context": null, "consequences": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Both directions of the directory now resolve.
    let (_, users) = send_as(&app, BOB, "GET", "/api/v1/users", None).await;
    assert_eq!(users["items"].as_array().unwrap().len(), 2);

    // Members don't manage: no edits, no invites, no removing others.
    let (status, _) = send_as(
        &app,
        BOB,
        "PATCH",
        &format!("/api/v1/groups/{gid}"),
        Some(json!([{ "set_name": "mine now" }])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send_as(
        &app,
        BOB,
        "POST",
        &format!("/api/v1/groups/{gid}/members"),
        Some(json!({ "handle": "admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The owner has no membership row to delete — removing them is 404.
    let owner = group_owner(&app, &gid).await;
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/groups/{gid}/members/{}", owner.as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Bob leaves; the door closes behind him.
    let (status, _) = send_as(
        &app,
        BOB,
        "DELETE",
        &format!("/api/v1/groups/{gid}/members/{bob}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send_as(&app, BOB, "GET", &format!("/api/v1/groups/{gid}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn group_owner(app: &axum::Router, gid: &str) -> serde_json::Value {
    let (_, group) = send(app, "GET", &format!("/api/v1/groups/{gid}"), None).await;
    group["owner"].clone()
}
