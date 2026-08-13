//! Group/project deletion over the HTTP surface: owner-only authority,
//! full cascade, the evidence-pinning conflict, and the sentinel guard
//! (testcontainers — needs Docker).

mod common;

use axum::http::StatusCode;
use common::{send, send_as, server};
use converge_storage::{
    Author, Decisions, Identity, Memberships, NewDecision, NewGroup, NewMessage, NewProject,
    NewSession, Projects, Scope, Sessions, Storage, Tokens, Users,
};

/// Seed a group (owned by `owner`) with one project; answer both ids.
async fn seed<S: Storage>(
    store: &S,
    owner: converge_storage::UserId,
    name: &str,
) -> (converge_storage::GroupId, converge_storage::ProjectId) {
    let group = store
        .group_add(
            owner,
            NewGroup {
                name: format!("{name}-group"),
                description: None,
                kind: converge_storage::GroupKind::Shared,
            },
        )
        .await
        .unwrap();
    let project = store
        .project_add(
            Scope::User(owner),
            NewProject {
                group_id: group,
                name: name.into(),
                description: None,
            },
        )
        .await
        .unwrap();
    (group, project)
}

/// One decision with a message-anchored evidence trail in `project`.
async fn record<S: Storage>(
    store: &S,
    user: converge_storage::UserId,
    project: converge_storage::ProjectId,
    title: &str,
) -> (converge_storage::DecisionId, converge_storage::MessageId) {
    let session = store
        .session_ensure(
            Scope::User(user),
            NewSession {
                project_id: project,
                kind: converge_storage::SessionKind::Transcript,
                external: format!("t-{title}"),
                title: title.into(),
            },
        )
        .await
        .unwrap();
    let message = store
        .message_add(
            Scope::User(user),
            session,
            vec![NewMessage {
                speaker: "test".into(),
                body: format!("the line that decided {title}"),
                sent_at: None,
            }],
        )
        .await
        .unwrap()[0];
    let decision = store
        .decision_add(
            Scope::User(user),
            NewDecision {
                project_id: project,
                status: converge_storage::DecisionStatus::Accepted,
                title: title.into(),
                summary: "s".into(),
                context: None,
                consequences: None,
                alternatives: vec![],
                authors: vec![Author::User(user)],
                supersedes: vec![],
                evidence: vec![message],
            },
        )
        .await
        .unwrap();
    (decision, message)
}

#[tokio::test]
async fn project_delete_cascades_and_gates() {
    let (_pg, store, app) = server().await;
    let admin = store.user_lookup("admin").await.unwrap().remove(0).id;
    let (group, project) = seed(&store, admin, "web").await;
    let (decision, _) = record(&store, admin, project, "ship it").await;

    // A member sees the project but cannot delete it; an outsider
    // doesn't even see it.
    let beta = store
        .user_login(Identity {
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
    let (status, _) = send_as(
        &app,
        "cvg_beta",
        "DELETE",
        &format!("/api/v1/projects/{project}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    store
        .member_add(Scope::User(admin), group, beta)
        .await
        .unwrap();
    let (status, body) = send_as(
        &app,
        "cvg_beta",
        "DELETE",
        &format!("/api/v1/projects/{project}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // The owner deletes; the decision (and its whole trail) goes too.
    let (status, _) = send(&app, "DELETE", &format!("/api/v1/projects/{project}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let gone = store
        .decision_get(Scope::User(admin), decision)
        .await
        .unwrap();
    assert!(gone.is_none());
    let (status, _) = send(&app, "GET", &format!("/api/v1/projects/{project}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn evidence_elsewhere_pins_the_project() {
    let (_pg, store, app) = server().await;
    let admin = store.user_lookup("admin").await.unwrap().remove(0).id;
    // Evidence is same-group by domain rule, so the pinning case is two
    // projects sharing one group.
    let (group, keeper) = seed(&store, admin, "keeper").await;
    let donor = store
        .project_add(
            Scope::User(admin),
            NewProject {
                group_id: group,
                name: "donor".into(),
                description: None,
            },
        )
        .await
        .unwrap();

    // A decision in `keeper` anchored on a message living in `donor`:
    // the cross-project evidence that pins donor's session.
    let (_decision, message) = record(&store, admin, donor, "borrowed").await;
    let session = store
        .session_ensure(
            Scope::User(admin),
            NewSession {
                project_id: keeper,
                kind: converge_storage::SessionKind::Transcript,
                external: "keeper-session".into(),
                title: "keeper".into(),
            },
        )
        .await
        .unwrap();
    let _ = session; // keeper's own session is incidental
    store
        .decision_add(
            Scope::User(admin),
            NewDecision {
                project_id: keeper,
                status: converge_storage::DecisionStatus::Accepted,
                title: "leans on donor".into(),
                summary: "s".into(),
                context: None,
                consequences: None,
                alternatives: vec![],
                authors: vec![Author::User(admin)],
                supersedes: vec![],
                evidence: vec![message],
            },
        )
        .await
        .unwrap();

    // Deleting donor would take the evidenced message with it: refused.
    let (status, body) = send(&app, "DELETE", &format!("/api/v1/projects/{donor}"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Retire the leaning decision's project first; donor then deletes.
    let (status, _) = send(&app, "DELETE", &format!("/api/v1/projects/{keeper}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&app, "DELETE", &format!("/api/v1/projects/{donor}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn group_delete_takes_everything_but_not_the_sentinel() {
    let (_pg, store, app) = server().await;
    let admin = store.user_lookup("admin").await.unwrap().remove(0).id;
    let (group, project) = seed(&store, admin, "doomed").await;
    record(&store, admin, project, "gone with it").await;

    let (status, _) = send(&app, "DELETE", &format!("/api/v1/groups/{group}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&app, "GET", &format!("/api/v1/projects/{project}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The default workspace is infrastructure, not data.
    store.ensure_default_workspace().await.unwrap();
    let sentinel = "00000000000000000000000000";
    let (status, body) = send(&app, "DELETE", &format!("/api/v1/groups/{sentinel}"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
