//! Sessions, message streams, and decision→message evidence anchors
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    DecisionEdit, DecisionId, DecisionStatus, Decisions, GroupKind, Groups, MessageId, Messages,
    NewDecision, NewGroup, NewMessage, NewProject, NewSession, Pagination, ProjectId, Projects,
    SessionFilter, SessionId, SessionKind, Sessions, StoreError,
};
use converge_storage_postgres::PgStorage;
use time::OffsetDateTime;

async fn project(store: &PgStorage) -> ProjectId {
    let group = store
        .group_add(NewGroup {
            name: "g".into(),
            description: None,
            kind: GroupKind::Shared,
        })
        .await
        .unwrap();
    store
        .project_add(NewProject {
            group_id: group,
            name: "p".into(),
            description: None,
        })
        .await
        .unwrap()
}

fn session(project_id: ProjectId, external: &str, title: &str) -> NewSession {
    NewSession {
        project_id,
        kind: SessionKind::Transcript,
        external: external.into(),
        title: title.into(),
    }
}

fn message(speaker: &str, body: &str) -> NewMessage {
    NewMessage {
        speaker: speaker.into(),
        body: body.into(),
        sent_at: None,
    }
}

#[tokio::test]
async fn session_ensure_by_natural_key() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;
    let other_project = project(&store).await;

    // `(kind, external)` decides identity; the title refreshes…
    let first = store
        .session_ensure(session(project_id, "sess-1", "Early title"))
        .await
        .unwrap();
    let again = store
        .session_ensure(session(project_id, "sess-1", "Grown-up title"))
        .await
        .unwrap();
    assert_eq!(first, again);
    let got = store.session_get(first).await.unwrap().unwrap();
    assert_eq!(got.title, "Grown-up title");
    assert_eq!(got.kind, SessionKind::Transcript);
    assert_eq!(got.external, "sess-1");

    // …but the project binding stays as first created.
    let rehomed = store
        .session_ensure(session(other_project, "sess-1", "x"))
        .await
        .unwrap();
    assert_eq!(rehomed, first);
    assert_eq!(
        store.session_get(first).await.unwrap().unwrap().project_id,
        project_id
    );

    // Same external under a different kind is a different session.
    let slack = store
        .session_ensure(NewSession {
            kind: SessionKind::Slack,
            ..session(project_id, "sess-1", "thread")
        })
        .await
        .unwrap();
    assert_ne!(slack, first);

    // Filters narrow; unknown id reads absent.
    let of_project = store
        .session_list(
            SessionFilter {
                project: Some(project_id),
                kind: Some(SessionKind::Transcript),
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(of_project.len(), 1);
    assert_eq!(of_project[0].id, first);
    assert!(store.session_get(SessionId::new()).await.unwrap().is_none());
}

#[tokio::test]
async fn streams_append_in_order() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;
    let sid = store
        .session_ensure(session(project_id, "s", "s"))
        .await
        .unwrap();

    // Two batches: seq continues, order is conversation order.
    let when = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let first = store
        .message_add(
            sid,
            vec![
                message("maksim", "should we split the trait?"),
                NewMessage {
                    sent_at: Some(when),
                    ..message("claude", "per-resource traits compose better")
                },
            ],
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    let second = store
        .message_add(sid, vec![message("maksim", "agreed, do it")])
        .await
        .unwrap();
    assert_eq!(second.len(), 1);

    let stream = store
        .message_list(sid, Pagination::default())
        .await
        .unwrap();
    assert_eq!(stream.len(), 3);
    assert_eq!(stream[0].seq, 0);
    assert_eq!(stream[2].seq, 2);
    assert_eq!(stream[0].speaker, "maksim");
    assert_eq!(stream[1].sent_at, Some(when));
    assert_eq!(stream[2].body, "agreed, do it");

    // Forward cursor: strictly after the given message.
    let rest = store
        .message_list(
            sid,
            Pagination {
                limit: Some(10),
                cursor: Some(stream[0].id),
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[0].id, stream[1].id);

    // Unknown session: NotFound on append, empty on read.
    assert!(matches!(
        store
            .message_add(SessionId::new(), vec![message("x", "y")])
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(
        store
            .message_list(SessionId::new(), Pagination::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn evidence_anchors_decisions_to_messages() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;
    let sid = store
        .session_ensure(session(project_id, "s", "s"))
        .await
        .unwrap();
    let messages = store
        .message_add(
            sid,
            vec![message("maksim", "context"), message("claude", "the call")],
        )
        .await
        .unwrap();

    // Anchors at capture time round-trip on get and list; duplicates
    // collapse (a set, like authorship).
    let decision = store
        .decision_add(NewDecision {
            project_id,
            status: DecisionStatus::Accepted,
            title: "t".into(),
            summary: String::new(),
            context: None,
            consequences: None,
            alternatives: Vec::new(),
            authors: Vec::new(),
            supersedes: Vec::new(),
            evidence: vec![messages[1], messages[1]],
        })
        .await
        .unwrap();
    let got = store.decision_get(decision).await.unwrap().unwrap();
    assert_eq!(got.evidence, vec![messages[1]]);
    let listed = store
        .decision_list(Default::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(listed[0].evidence, vec![messages[1]]);

    // The edit batch grows and shrinks the set; unknown anchors are the
    // caller's error.
    store
        .decision_edit(
            decision,
            vec![
                DecisionEdit::AddEvidence(messages[0]),
                DecisionEdit::RemoveEvidence(messages[1]),
            ],
        )
        .await
        .unwrap();
    let got = store.decision_get(decision).await.unwrap().unwrap();
    assert_eq!(got.evidence, vec![messages[0]]);
    assert!(matches!(
        store
            .decision_edit(decision, vec![DecisionEdit::AddEvidence(MessageId::new())])
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_add(NewDecision {
                evidence: vec![MessageId::new()],
                ..NewDecision {
                    project_id,
                    status: DecisionStatus::Accepted,
                    title: "x".into(),
                    summary: String::new(),
                    context: None,
                    consequences: None,
                    alternatives: Vec::new(),
                    authors: Vec::new(),
                    supersedes: Vec::new(),
                    evidence: Vec::new(),
                }
            })
            .await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn sources_derive_windows_around_anchors() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;

    // Session one: eight messages, anchors at seq 1 and 6 — two disjoint
    // windows. Session two: one anchored message of three.
    let s1 = store
        .session_ensure(session(project_id, "s1", "first"))
        .await
        .unwrap();
    let m1 = store
        .message_add(s1, (0..8).map(|i| message("a", &format!("m{i}"))).collect())
        .await
        .unwrap();
    let s2 = store
        .session_ensure(session(project_id, "s2", "second"))
        .await
        .unwrap();
    let m2 = store
        .message_add(s2, (0..3).map(|i| message("b", &format!("n{i}"))).collect())
        .await
        .unwrap();

    let decision = store
        .decision_add(NewDecision {
            project_id,
            status: DecisionStatus::Accepted,
            title: "t".into(),
            summary: String::new(),
            context: None,
            consequences: None,
            alternatives: Vec::new(),
            authors: Vec::new(),
            supersedes: Vec::new(),
            evidence: vec![m1[1], m1[6], m2[0]],
        })
        .await
        .unwrap();

    let sources = store.decision_sources(decision).await.unwrap().unwrap();
    assert_eq!(sources.len(), 2);
    // Newest session first (like every other list).
    assert_eq!(sources[0].session.id, s2);
    assert_eq!(sources[1].session.id, s1);

    // s2: anchor at seq 0 → window clamps to [0, 2].
    let seqs: Vec<i32> = sources[0].messages.iter().map(|m| m.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
    assert_eq!(sources[0].anchors, vec![m2[0]]);

    // s1: anchors 1 and 6 → windows [0..=3] and [4..=8), seq 0..=3 ∪ 4..=8.
    let seqs: Vec<i32> = sources[1].messages.iter().map(|m| m.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(sources[1].anchors, vec![m1[1], m1[6]]);

    // No evidence → empty; unknown decision → None.
    let bare = store
        .decision_add(NewDecision {
            project_id,
            status: DecisionStatus::Accepted,
            title: "bare".into(),
            summary: String::new(),
            context: None,
            consequences: None,
            alternatives: Vec::new(),
            authors: Vec::new(),
            supersedes: Vec::new(),
            evidence: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        store.decision_sources(bare).await.unwrap().unwrap(),
        Vec::new()
    );
    assert!(
        store
            .decision_sources(DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
}
