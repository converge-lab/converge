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
    let mut projects = Vec::new();
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
        projects.push(p["id"].as_str().unwrap().to_string());
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

    // `since` pages forward, oldest first, through the shared filter —
    // no route code knows about it. Both ways at once is a 400.
    let (_, second) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": c, "targets": [a], "kind": "duplication", "tier": "watch",
            "title": "c repeats a", "text": "same call, other project",
            "consequence": null, "recommendation": null,
            "produced_by": { "user": user },
        })),
    )
    .await;
    let second = second["id"].as_str().unwrap().to_string();
    // ULID strings order like the ids they spell.
    let (lo, hi) = if id < second {
        (&id, &second)
    } else {
        (&second, &id)
    };
    let (status, page) = send(&app, "GET", &format!("/api/v1/signals?since={lo}"), None).await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], json!(hi));
    let (status, _) = send(
        &app,
        "GET",
        &format!("/api/v1/signals?since={lo}&cursor={hi}"),
        None,
    )
    .await;
    assert_eq!(status, 400);

    // Receipts: what a session was shown stops being unseen for this
    // user, in any session, and the rest stays.
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/signals/receipts",
        Some(json!({ "session": "sess-1", "harness": "codex", "signal_ids": [id] })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, page) = send(&app, "GET", "/api/v1/signals?unseen=true", None).await;
    let unseen: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(unseen, vec![second.as_str()]);

    // The poll's endpoint: the receipts above opened sess-1's row, so
    // nothing recorded before it is handed out; what comes after arrives
    // once.
    let claim = |session: &'static str| {
        let app = app.clone();
        async move {
            let (status, got) = send(
                &app,
                "POST",
                "/api/v1/signals/claims",
                Some(json!({ "session": session, "harness": "codex", "limit": 3 })),
            )
            .await;
            assert_eq!(status, 200, "{got}");
            got.as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(claim("sess-1").await, Vec::<String>::new());
    let (_, third) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": a, "targets": [c], "kind": "divergence", "tier": "coordinate",
            "title": "a and c drift", "text": "two answers to one question",
            "consequence": null, "recommendation": null,
            "produced_by": { "user": user },
        })),
    )
    .await;
    let third = third["id"].as_str().unwrap().to_string();
    assert_eq!(claim("sess-1").await, vec![third]);
    assert_eq!(claim("sess-1").await, Vec::<String>::new());
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/signals/claims",
        Some(json!({ "session": " " })),
    )
    .await;
    assert_eq!(status, 400);

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

    // A claim names the project its session is working in, and hears
    // only about that one. Both sessions open their rows first, so what
    // follows is a real hand-out rather than a first sight.
    let claim_in = |session: &'static str, project: String| {
        let app = app.clone();
        async move {
            let (status, got) = send(
                &app,
                "POST",
                "/api/v1/signals/claims",
                Some(json!({
                    "session": session, "harness": "claude",
                    "project": project, "limit": 3,
                })),
            )
            .await;
            assert_eq!(status, 200, "{got}");
            got.as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert!(claim_in("sess-a", projects[0].clone()).await.is_empty());
    assert!(claim_in("sess-c", projects[2].clone()).await.is_empty());
    let (_, fourth) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": a, "targets": [b], "kind": "divergence", "tier": "conflict",
            "title": "a and b disagree", "text": "one of them has to move",
            "consequence": null, "recommendation": null,
            "produced_by": { "user": user },
        })),
    )
    .await;
    let fourth = fourth["id"].as_str().unwrap().to_string();
    // c's project is on neither end: it hears nothing, and because it
    // hears nothing it consumes nothing.
    assert_eq!(
        claim_in("sess-c", projects[2].clone()).await,
        Vec::<String>::new()
    );
    assert_eq!(
        claim_in("sess-a", projects[0].clone()).await,
        vec![fourth.clone()]
    );
    // b's project is the other end of a signal raised over in c's:
    // its own session hears it, because reach is either end.
    assert!(claim_in("sess-b", projects[1].clone()).await.is_empty());
    let (_, fifth) = send(
        &app,
        "POST",
        "/api/v1/signals",
        Some(json!({
            "source": c, "targets": [b], "kind": "dependency", "tier": "watch",
            "title": "b leans on c", "text": "c's shape decides b's",
            "consequence": null, "recommendation": null,
            "produced_by": { "user": user },
        })),
    )
    .await;
    let fifth = fifth["id"].as_str().unwrap().to_string();
    assert_eq!(claim_in("sess-b", projects[1].clone()).await, vec![fifth]);
}
