//! Round-trip tests for the decision methods, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use common::{newest_first, store};
use converge_storage::{
    Alternative, Author, DecisionEdit, DecisionFilter, DecisionId, DecisionStatus, Decisions,
    GroupId, GroupKind, Groups, Identity, NewDecision, NewGroup, NewProject, Pagination, ProjectId,
    Projects, Related, Scope, StoreError, UserId, Users,
};
use converge_storage_postgres::PgStorage;

/// A group + project to hang decisions on (owned by a bootstrap user;
/// `user_login` is idempotent, so repeated seeding reuses the same owner).
async fn seed_project(store: &PgStorage) -> (GroupId, ProjectId, UserId) {
    let owner = store
        .user_login(Identity {
            provider: "local".into(),
            subject: "test".into(),
            handle: "test".into(),
            name: "Test".into(),
        })
        .await
        .unwrap();
    let group = store
        .group_add(
            owner,
            NewGroup {
                name: "test group".into(),
                description: None,
                kind: GroupKind::Shared,
            },
        )
        .await
        .unwrap();
    let project = store
        .project_add(
            Scope::System,
            NewProject {
                group_id: group,
                name: "test project".into(),
                description: None,
                repository: None,
            },
        )
        .await
        .unwrap();
    (group, project, owner)
}

fn decision(project: ProjectId, by: UserId, title: &str) -> NewDecision {
    NewDecision {
        project_id: project,
        status: DecisionStatus::Accepted,
        title: title.into(),
        summary: "because it won".into(),
        context: Some("the setting".into()),
        consequences: None,
        alternatives: vec![Alternative {
            option: "the other way".into(),
            why_rejected: "slower".into(),
        }],
        authors: vec![Author::User(by)],
        supersedes: Vec::new(),
        evidence: Vec::new(),
        code_evidence: Vec::new(),
    }
}

#[tokio::test]
async fn an_author_is_required() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let mut new = decision(project, me, "nobody's");
    new.authors.clear();
    match store.decision_add(Scope::System, new).await {
        Err(StoreError::Invalid(m)) => assert!(m.contains("author"), "{m}"),
        other => panic!("expected Invalid(author), got {other:?}"),
    }
}

#[tokio::test]
async fn round_trip() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;

    let id = store
        .decision_add(Scope::System, decision(project, me, "adopt X"))
        .await
        .unwrap();
    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .expect("stored decision");
    assert_eq!(got.id, id);
    assert_eq!(got.project_id, project);
    assert_eq!(got.status, DecisionStatus::Accepted);
    assert_eq!(got.title, "adopt X");
    assert_eq!(got.summary, "because it won");
    assert_eq!(got.context.as_deref(), Some("the setting"));
    assert_eq!(got.consequences, None);
    assert_eq!(got.alternatives.len(), 1);
    assert_eq!(got.alternatives[0].option, "the other way");
    assert_eq!(got.authors, vec![Author::User(me)]);

    assert!(
        store
            .decision_get(Scope::System, DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn add_with_proposed_status() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;

    let id = store
        .decision_add(
            Scope::System,
            NewDecision {
                status: DecisionStatus::Proposed,
                ..decision(project, me, "try Z")
            },
        )
        .await
        .unwrap();

    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.status, DecisionStatus::Proposed);
}

#[tokio::test]
async fn add_with_rejected_status() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;

    let id = store
        .decision_add(
            Scope::System,
            NewDecision {
                status: DecisionStatus::Rejected,
                ..decision(project, me, "skip W")
            },
        )
        .await
        .unwrap();

    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.status, DecisionStatus::Rejected);
}

#[tokio::test]
async fn list_filters() {
    let (_pg, store) = store().await;
    let (_, a, _me) = seed_project(&store).await;
    let (group_b, b, me) = seed_project(&store).await;

    let d1 = store
        .decision_add(Scope::System, decision(a, me, "one"))
        .await
        .unwrap();
    let d2 = store
        .decision_add(
            Scope::System,
            NewDecision {
                status: DecisionStatus::Proposed,
                ..decision(a, me, "two")
            },
        )
        .await
        .unwrap();
    let d3 = store
        .decision_add(Scope::System, decision(b, me, "three"))
        .await
        .unwrap();
    // Ordered expectations are computed (`common::newest_first`): the
    // list contract is `order by id desc`, which tracks creation order
    // only to the ULID's millisecond.
    let by_id = newest_first(&[d1, d2, d3]);

    // No filter: everything, id-descending.
    let all = store
        .decision_list(
            Scope::System,
            DecisionFilter::default(),
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(all.iter().map(|d| d.id).collect::<Vec<_>>(), by_id);

    let of_a = store
        .decision_list(
            Scope::System,
            DecisionFilter {
                project: Some(a),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        of_a.iter().map(|d| d.id).collect::<Vec<_>>(),
        newest_first(&[d1, d2])
    );

    let of_group_b = store
        .decision_list(
            Scope::System,
            DecisionFilter {
                group: Some(group_b),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        of_group_b.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![d3]
    );

    let proposed = store
        .decision_list(
            Scope::System,
            DecisionFilter {
                status: Some(DecisionStatus::Proposed),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(proposed.iter().map(|d| d.id).collect::<Vec<_>>(), vec![d2]);

    let latest = store
        .decision_list(
            Scope::System,
            DecisionFilter::default(),
            Pagination {
                limit: Some(2),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(latest.iter().map(|d| d.id).collect::<Vec<_>>(), by_id[..2]);

    // Cursor paging: strictly older (by id) than the cursor, descending —
    // the expected page is computed from the same total order.
    let cursor = by_id[1];
    let paged = store
        .decision_list(
            Scope::System,
            DecisionFilter::default(),
            Pagination {
                limit: Some(2),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
    assert_eq!(paged.iter().map(|d| d.id).collect::<Vec<_>>(), by_id[2..]);
}

#[tokio::test]
async fn edit_batch() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let id = store
        .decision_add(Scope::System, decision(project, me, "draft Y"))
        .await
        .unwrap();

    store
        .decision_edit(
            Scope::System,
            id,
            vec![
                DecisionEdit::SetTitle("adopt Y".into()),
                DecisionEdit::SetStatus(DecisionStatus::Rejected),
                DecisionEdit::SetContext(None),
                DecisionEdit::SetAlternatives(Vec::new()),
            ],
        )
        .await
        .unwrap();

    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.title, "adopt Y");
    assert_eq!(got.status, DecisionStatus::Rejected);
    assert_eq!(got.context, None);
    assert!(got.alternatives.is_empty());
    // Untouched fields stay.
    assert_eq!(got.summary, "because it won");

    // Editing a missing decision is NotFound.
    let missing = store
        .decision_edit(
            Scope::System,
            DecisionId::new(),
            vec![DecisionEdit::SetTitle("x".into())],
        )
        .await;
    assert!(matches!(missing, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn supersession_derives_status() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;

    let old = store
        .decision_add(Scope::System, decision(project, me, "v1"))
        .await
        .unwrap();
    let new = store
        .decision_add(
            Scope::System,
            NewDecision {
                supersedes: vec![old],
                ..decision(project, me, "v2")
            },
        )
        .await
        .unwrap();

    // The stored status of `old` is untouched, but it *reads* superseded.
    let got_old = store
        .decision_get(Scope::System, old)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_old.status, DecisionStatus::Superseded);
    let got_new = store
        .decision_get(Scope::System, new)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_new.status, DecisionStatus::Accepted);

    // Edges, both directions.
    let edges_old = store
        .decision_edges(Scope::System, old)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edges_old.superseded_by, vec![new]);
    assert!(edges_old.supersedes.is_empty());
    let edges_new = store
        .decision_edges(Scope::System, new)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edges_new.supersedes, vec![old]);

    // The list status filter matches the derived status.
    let superseded = store
        .decision_list(
            Scope::System,
            DecisionFilter {
                status: Some(DecisionStatus::Superseded),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        superseded.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![old]
    );

    // Removing the edge restores the stored status.
    store
        .decision_edit(
            Scope::System,
            new,
            vec![DecisionEdit::RemoveSupersedes(old)],
        )
        .await
        .unwrap();
    let restored = store
        .decision_get(Scope::System, old)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.status, DecisionStatus::Accepted);

    // Edges of a missing decision → None.
    assert!(
        store
            .decision_edges(Scope::System, DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn related_upsert() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let a = store
        .decision_add(Scope::System, decision(project, me, "a"))
        .await
        .unwrap();
    let b = store
        .decision_add(Scope::System, decision(project, me, "b"))
        .await
        .unwrap();

    // Re-adding an existing cross-ref updates `why` (upsert, no duplicate).
    store
        .decision_edit(
            Scope::System,
            a,
            vec![DecisionEdit::AddRelated {
                to: b,
                why: Some("first".into()),
            }],
        )
        .await
        .unwrap();
    store
        .decision_edit(
            Scope::System,
            a,
            vec![DecisionEdit::AddRelated {
                to: b,
                why: Some("updated".into()),
            }],
        )
        .await
        .unwrap();

    let ea = store
        .decision_edges(Scope::System, a)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ea.related_to,
        vec![Related {
            id: b,
            why: Some("updated".into())
        }]
    );
    assert!(ea.related_by.is_empty());
    let eb = store
        .decision_edges(Scope::System, b)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        eb.related_by,
        vec![Related {
            id: a,
            why: Some("updated".into())
        }]
    );
    assert!(eb.related_to.is_empty());

    // Removal is idempotent.
    store
        .decision_edit(Scope::System, a, vec![DecisionEdit::RemoveRelated(b)])
        .await
        .unwrap();
    store
        .decision_edit(Scope::System, a, vec![DecisionEdit::RemoveRelated(b)])
        .await
        .unwrap();
    assert!(
        store
            .decision_edges(Scope::System, a)
            .await
            .unwrap()
            .unwrap()
            .related_to
            .is_empty()
    );
}

#[tokio::test]
async fn graph_guards() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let a = store
        .decision_add(Scope::System, decision(project, me, "a"))
        .await
        .unwrap();

    // Self-loops are rejected.
    assert!(matches!(
        store
            .decision_edit(Scope::System, a, vec![DecisionEdit::AddSupersedes(a)])
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_edit(
                Scope::System,
                a,
                vec![DecisionEdit::AddRelated { to: a, why: None }]
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // Superseded is derived — it can't be set or created.
    assert!(matches!(
        store
            .decision_edit(
                Scope::System,
                a,
                vec![DecisionEdit::SetStatus(DecisionStatus::Superseded)]
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_add(
                Scope::System,
                NewDecision {
                    status: DecisionStatus::Superseded,
                    ..decision(project, me, "born superseded")
                }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A creation-time edge to a missing decision fails whole (FK, atomic).
    let orphan_edge = NewDecision {
        supersedes: vec![DecisionId::new()],
        ..decision(project, me, "dangling")
    };
    assert!(matches!(
        store.decision_add(Scope::System, orphan_edge).await,
        Err(StoreError::Invalid(_))
    ));
    let titles: Vec<String> = store
        .decision_list(
            Scope::System,
            DecisionFilter::default(),
            Pagination::default(),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect();
    assert!(!titles.contains(&"dangling".to_string()));
}

#[tokio::test]
async fn edit_batch_is_atomic() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let id = store
        .decision_add(Scope::System, decision(project, me, "stable"))
        .await
        .unwrap();

    // Postgres rejects NUL bytes in text, so the second op fails — and the
    // already-applied first op must roll back with it.
    let failed = store
        .decision_edit(
            Scope::System,
            id,
            vec![
                DecisionEdit::SetSummary("half-applied".into()),
                DecisionEdit::SetTitle("bad\0title".into()),
            ],
        )
        .await;
    assert!(failed.is_err());

    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.summary, "because it won");
    assert_eq!(got.title, "stable");
}

#[tokio::test]
async fn add_guards() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;

    // Authorship isn't wired yet — must fail loud, not drop silently.
    let mut authored = decision(project, me, "authored");
    authored.authors.push(Author::User(UserId::new()));
    assert!(matches!(
        store.decision_add(Scope::System, authored).await,
        Err(StoreError::Invalid(_))
    ));

    // Unknown project: FK violation surfaces as Invalid.
    let orphan = decision(ProjectId::new(), me, "orphan");
    assert!(matches!(
        store.decision_add(Scope::System, orphan).await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn code_anchors_are_validated_stored_and_edited() {
    use converge_storage::{CodeAnchor, EXCERPT_LINES};
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let sha = "a".repeat(40);
    let excerpt = "let x = 1;\nlet y = x + 1;\n".to_string();
    let anchor = CodeAnchor {
        commit: sha.clone(),
        path: "crates/x/src/lib.rs".into(),
        lines: (10, 11),
        digest: CodeAnchor::digest_of(&excerpt),
        excerpt,
    };

    // Invariants fail loudly, each naming what is wrong.
    for (broken, needle) in [
        (
            CodeAnchor {
                commit: "abc".into(),
                ..anchor.clone()
            },
            "40-hex",
        ),
        (
            CodeAnchor {
                path: "/abs/path.rs".into(),
                ..anchor.clone()
            },
            "repository-relative",
        ),
        (
            CodeAnchor {
                lines: (11, 10),
                ..anchor.clone()
            },
            "start <= end",
        ),
        (
            CodeAnchor {
                lines: (1, EXCERPT_LINES as u32 + 1),
                ..anchor.clone()
            },
            "at most",
        ),
        (
            CodeAnchor {
                digest: "0".repeat(64),
                ..anchor.clone()
            },
            "digest",
        ),
    ] {
        let mut new = decision(project, me, "broken");
        new.code_evidence = vec![broken];
        match store.decision_add(Scope::System, new).await {
            Err(StoreError::Invalid(m)) => assert!(m.contains(needle), "{m} ∌ {needle}"),
            other => panic!("expected Invalid({needle}), got {other:?}"),
        }
    }

    // Stored with the decision, read back whole, a set.
    let mut new = decision(project, me, "anchored");
    new.code_evidence = vec![anchor.clone(), anchor.clone()];
    let id = store.decision_add(Scope::System, new).await.unwrap();
    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.code_evidence, vec![anchor.clone()]);
    let listed = store
        .decision_list(
            Scope::System,
            DecisionFilter {
                project: Some(project),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(listed[0].code_evidence, vec![anchor.clone()]);

    // Edits add and drop by key; the excerpt is immutable — re-adding
    // the same key with other text changes nothing.
    let second = CodeAnchor {
        path: "crates/x/src/other.rs".into(),
        lines: (1, 1),
        excerpt: "fn f() {}\n".into(),
        digest: CodeAnchor::digest_of("fn f() {}\n"),
        ..anchor.clone()
    };
    store
        .decision_edit(
            Scope::System,
            id,
            vec![
                DecisionEdit::AddCodeEvidence(second.clone()),
                DecisionEdit::AddCodeEvidence(CodeAnchor {
                    excerpt: "let x = 2;\nlet y = 3;\n".into(),
                    digest: CodeAnchor::digest_of("let x = 2;\nlet y = 3;\n"),
                    ..anchor.clone()
                }),
            ],
        )
        .await
        .unwrap();
    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.code_evidence, vec![anchor.clone(), second.clone()]);
    store
        .decision_edit(
            Scope::System,
            id,
            vec![DecisionEdit::RemoveCodeEvidence {
                commit: sha.clone(),
                path: anchor.path.clone(),
                lines: anchor.lines,
            }],
        )
        .await
        .unwrap();
    let got = store
        .decision_get(Scope::System, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.code_evidence, vec![second]);
}

#[tokio::test]
async fn receipts_decide_what_is_unseen() {
    let (_pg, store) = store().await;
    let (_, project, me) = seed_project(&store).await;
    let first = store
        .decision_add(Scope::System, decision(project, me, "first"))
        .await
        .unwrap();
    let second = store
        .decision_add(Scope::System, decision(project, me, "second"))
        .await
        .unwrap();
    let unseen = |scope: Scope| {
        let store = store.clone();
        async move {
            let mut ids: Vec<_> = store
                .decision_list(
                    scope,
                    DecisionFilter {
                        unseen: true,
                        ..Default::default()
                    },
                    Pagination::default(),
                )
                .await
                .unwrap()
                .into_iter()
                .map(|d| d.id)
                .collect();
            ids.sort_unstable();
            ids
        }
    };
    let mut both = vec![first, second];
    both.sort_unstable();
    assert_eq!(unseen(Scope::User(me)).await, both);

    // Shown in a harness session: seen, in every session after.
    store
        .decision_receive(Scope::User(me), "s1", Some("claude"), &[first])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(me)).await, vec![second]);
    // Read on the web is the same receipt with an empty session.
    store
        .decision_receive(Scope::User(me), "", None, &[second])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(me)).await, vec![]);
    // Idempotent, and the unfiltered listing still holds everything.
    store
        .decision_receive(Scope::User(me), "", None, &[second, second])
        .await
        .unwrap();
    assert_eq!(
        store
            .decision_list(
                Scope::User(me),
                DecisionFilter::default(),
                Pagination::default()
            )
            .await
            .unwrap()
            .len(),
        2
    );
    // System has no receipts: it sees everything, and cannot record one.
    assert_eq!(unseen(Scope::System).await, both);
    assert!(matches!(
        store
            .decision_receive(Scope::System, "", None, &[first])
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A stranger's receipt says nothing about what this user has seen,
    // and a decision they cannot see is not receipted at all.
    let stranger = store
        .user_login(Identity {
            provider: "local".into(),
            subject: "stranger".into(),
            handle: "stranger".into(),
            name: "Stranger".into(),
        })
        .await
        .unwrap();
    store
        .decision_receive(Scope::User(stranger), "", None, &[first])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(stranger)).await, vec![]);
}
