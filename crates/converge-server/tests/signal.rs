//! `/api/v1/signals` — the observation resource over REST
//! (testcontainers — needs Docker).

mod common;

use common::{send, server};
use serde_json::json;

#[tokio::test]
async fn signal_round_trip() {
    let (_pg, _store, app) = server().await;

    // Seed: a group, two projects, three decisions.
    let (_, me) = send(&app, "GET", "/api/v1/users/me", None).await;
    let user = me["id"].as_str().unwrap().to_string();
    let (_, group) = send(
        &app,
        "POST",
        "/api/v1/groups",
        Some(json!({ "name": "team", "kind": "shared" })),
    )
    .await;
    let gid = group["id"].as_str().unwrap();
    let mut decisions = Vec::new();
    for (project, title) in [("server", "a"), ("billing", "b"), ("billing", "c")] {
        let (_, p) = send(
            &app,
            "POST",
            "/api/v1/projects",
            Some(json!({ "group_id": gid, "name": format!("{project}-{title}") })),
        )
        .await;
        let (_, d) = send(
            &app,
            "POST",
            "/api/v1/decisions",
            Some(json!({
                "project_id": p["id"], "status": "accepted",
                "title": title, "summary": "",
                "context": null, "consequences": null,
            })),
        )
        .await;
        decisions.push(d["id"].as_str().unwrap().to_string());
    }
    let (a, b, c) = (&decisions[0], &decisions[1], &decisions[2]);

    // Record: born proposed, targets a set.
    let (status, created) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": a,
            "targets": [b, c],
            "kind": "dependency",
            "tier": "conflict",
            "title": "auth API change breaks billing",
            "text": "billing decision b consumes the endpoint a reshapes",
            "consequence": "billing breaks on deploy",
            "recommendation": "coordinate the rollout",
            "produced_by": { "user": user },
        })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, got) = send(&app, "GET", &format!("/api/v1/signals/{id}"), None).await;
    assert_eq!(status, 200);
    assert_eq!(got["source"], *a);
    assert_eq!(got["targets"].as_array().unwrap().len(), 2);
    assert_eq!(got["status"], "proposed");
    assert_eq!(got["tier"], "conflict");
    assert_eq!(got["resolved_by"], serde_json::Value::Null);

    // The duplicate pair conflicts (409 via the shared error mapping).
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": a, "targets": [b], "kind": "dependency", "tier": "watch",
            "title": "again", "text": "again",
            "consequence": null, "recommendation": null,
            "produced_by": { "user": user },
        })),
    )
    .await;
    assert_eq!(status, 409);

    // List narrows by tier and decision (either end).
    let (_, page) = send(
        &app,
        "GET",
        &format!("/api/v1/signals?decision={b}&tier=conflict"),
        None,
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    let (_, none) = send(&app, "GET", "/api/v1/signals?tier=watch", None).await;
    assert_eq!(none["items"].as_array().unwrap().len(), 0);

    // The decision projection: bound by the path, parent must exist.
    let (status, page) = send(&app, "GET", &format!("/api/v1/decisions/{b}/signals"), None).await;
    assert_eq!(status, 200);
    assert_eq!(page["items"][0]["id"], json!(id));
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{b}/signals?decision={a}"),
        None,
    )
    .await;
    assert_eq!(status, 400, "path-bound filter params are rejected");
    let ghost = converge_storage::DecisionId::new();
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/decisions/{ghost}/signals"),
        None,
    )
    .await;
    assert_eq!(status, 404);

    // Resolve: confirmed, judge stamped; `proposed` rejected.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/signals/{id}"),
        Some(json!({ "status": "confirmed", "by": { "user": user } })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, got) = send(&app, "GET", &format!("/api/v1/signals/{id}"), None).await;
    assert_eq!(got["status"], "confirmed");
    assert_eq!(got["resolved_by"]["user"], json!(user));
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/signals/{id}"),
        Some(json!({ "status": "proposed", "by": { "user": user } })),
    )
    .await;
    assert_eq!(status, 400);
}
