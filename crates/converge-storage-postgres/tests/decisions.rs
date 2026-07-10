//! Round-trip tests for the decision methods, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    Alternative, Author, DecisionEdit, DecisionFilter, DecisionId, DecisionStatus, Decisions,
    GroupId, GroupKind, Groups, NewDecision, NewGroup, NewProject, Pagination, ProjectId, Projects,
    Related, StoreError, UserId,
};
use converge_storage_postgres::PgStorage;

/// A group + project to hang decisions on.
async fn seed_project(store: &PgStorage) -> (GroupId, ProjectId) {
    let group = store
        .group_add(NewGroup {
            name: "test group".into(),
            description: None,
            kind: GroupKind::Shared,
        })
        .await
        .unwrap();
    let project = store
        .project_add(NewProject {
            group_id: group,
            name: "test project".into(),
            description: None,
        })
        .await
        .unwrap();
    (group, project)
}

fn decision(project: ProjectId, title: &str) -> NewDecision {
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
        authors: Vec::new(),
        supersedes: Vec::new(),
        evidence: Vec::new(),
    }
}

#[tokio::test]
async fn round_trip() {
    let (_pg, store) = store().await;
    let (_, project) = seed_project(&store).await;

    let id = store
        .decision_add(decision(project, "adopt X"))
        .await
        .unwrap();
    let got = store
        .decision_get(id)
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
    assert!(got.authors.is_empty());

    assert!(
        store
            .decision_get(DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn list_filters() {
    let (_pg, store) = store().await;
    let (_, a) = seed_project(&store).await;
    let (group_b, b) = seed_project(&store).await;

    let d1 = store.decision_add(decision(a, "one")).await.unwrap();
    let d2 = store
        .decision_add(NewDecision {
            status: DecisionStatus::Proposed,
            ..decision(a, "two")
        })
        .await
        .unwrap();
    let d3 = store.decision_add(decision(b, "three")).await.unwrap();

    // No filter: everything, newest first (ULID = time order).
    let all = store
        .decision_list(DecisionFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(
        all.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![d3, d2, d1]
    );

    let of_a = store
        .decision_list(
            DecisionFilter {
                project: Some(a),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(of_a.iter().map(|d| d.id).collect::<Vec<_>>(), vec![d2, d1]);

    let of_group_b = store
        .decision_list(
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
            DecisionFilter::default(),
            Pagination {
                limit: Some(2),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        latest.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![d3, d2]
    );

    // Cursor paging: strictly older than the cursor, newest first.
    let paged = store
        .decision_list(
            DecisionFilter::default(),
            Pagination {
                limit: Some(2),
                cursor: Some(d2),
            },
        )
        .await
        .unwrap();
    assert_eq!(paged.iter().map(|d| d.id).collect::<Vec<_>>(), vec![d1]);
}

#[tokio::test]
async fn edit_batch() {
    let (_pg, store) = store().await;
    let (_, project) = seed_project(&store).await;
    let id = store
        .decision_add(decision(project, "draft Y"))
        .await
        .unwrap();

    store
        .decision_edit(
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

    let got = store.decision_get(id).await.unwrap().unwrap();
    assert_eq!(got.title, "adopt Y");
    assert_eq!(got.status, DecisionStatus::Rejected);
    assert_eq!(got.context, None);
    assert!(got.alternatives.is_empty());
    // Untouched fields stay.
    assert_eq!(got.summary, "because it won");

    // Editing a missing decision is NotFound.
    let missing = store
        .decision_edit(DecisionId::new(), vec![DecisionEdit::SetTitle("x".into())])
        .await;
    assert!(matches!(missing, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn supersession_derives_status() {
    let (_pg, store) = store().await;
    let (_, project) = seed_project(&store).await;

    let old = store.decision_add(decision(project, "v1")).await.unwrap();
    let new = store
        .decision_add(NewDecision {
            supersedes: vec![old],
            ..decision(project, "v2")
        })
        .await
        .unwrap();

    // The stored status of `old` is untouched, but it *reads* superseded.
    let got_old = store.decision_get(old).await.unwrap().unwrap();
    assert_eq!(got_old.status, DecisionStatus::Superseded);
    let got_new = store.decision_get(new).await.unwrap().unwrap();
    assert_eq!(got_new.status, DecisionStatus::Accepted);

    // Edges, both directions.
    let edges_old = store.decision_edges(old).await.unwrap().unwrap();
    assert_eq!(edges_old.superseded_by, vec![new]);
    assert!(edges_old.supersedes.is_empty());
    let edges_new = store.decision_edges(new).await.unwrap().unwrap();
    assert_eq!(edges_new.supersedes, vec![old]);

    // The list status filter matches the derived status.
    let superseded = store
        .decision_list(
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
        .decision_edit(new, vec![DecisionEdit::RemoveSupersedes(old)])
        .await
        .unwrap();
    let restored = store.decision_get(old).await.unwrap().unwrap();
    assert_eq!(restored.status, DecisionStatus::Accepted);

    // Edges of a missing decision → None.
    assert!(
        store
            .decision_edges(DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn related_upsert() {
    let (_pg, store) = store().await;
    let (_, project) = seed_project(&store).await;
    let a = store.decision_add(decision(project, "a")).await.unwrap();
    let b = store.decision_add(decision(project, "b")).await.unwrap();

    // Re-adding an existing cross-ref updates `why` (upsert, no duplicate).
    store
        .decision_edit(
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
            a,
            vec![DecisionEdit::AddRelated {
                to: b,
                why: Some("updated".into()),
            }],
        )
        .await
        .unwrap();

    let ea = store.decision_edges(a).await.unwrap().unwrap();
    assert_eq!(
        ea.related_to,
        vec![Related {
            id: b,
            why: Some("updated".into())
        }]
    );
    assert!(ea.related_by.is_empty());
    let eb = store.decision_edges(b).await.unwrap().unwrap();
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
        .decision_edit(a, vec![DecisionEdit::RemoveRelated(b)])
        .await
        .unwrap();
    store
        .decision_edit(a, vec![DecisionEdit::RemoveRelated(b)])
        .await
        .unwrap();
    assert!(
        store
            .decision_edges(a)
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
    let (_, project) = seed_project(&store).await;
    let a = store.decision_add(decision(project, "a")).await.unwrap();

    // Self-loops are rejected.
    assert!(matches!(
        store
            .decision_edit(a, vec![DecisionEdit::AddSupersedes(a)])
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_edit(a, vec![DecisionEdit::AddRelated { to: a, why: None }])
            .await,
        Err(StoreError::Invalid(_))
    ));

    // Superseded is derived — it can't be set or created.
    assert!(matches!(
        store
            .decision_edit(a, vec![DecisionEdit::SetStatus(DecisionStatus::Superseded)])
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_add(NewDecision {
                status: DecisionStatus::Superseded,
                ..decision(project, "born superseded")
            })
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A creation-time edge to a missing decision fails whole (FK, atomic).
    let orphan_edge = NewDecision {
        supersedes: vec![DecisionId::new()],
        ..decision(project, "dangling")
    };
    assert!(matches!(
        store.decision_add(orphan_edge).await,
        Err(StoreError::Invalid(_))
    ));
    let titles: Vec<String> = store
        .decision_list(DecisionFilter::default(), Pagination::default())
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
    let (_, project) = seed_project(&store).await;
    let id = store
        .decision_add(decision(project, "stable"))
        .await
        .unwrap();

    // Postgres rejects NUL bytes in text, so the second op fails — and the
    // already-applied first op must roll back with it.
    let failed = store
        .decision_edit(
            id,
            vec![
                DecisionEdit::SetSummary("half-applied".into()),
                DecisionEdit::SetTitle("bad\0title".into()),
            ],
        )
        .await;
    assert!(failed.is_err());

    let got = store.decision_get(id).await.unwrap().unwrap();
    assert_eq!(got.summary, "because it won");
    assert_eq!(got.title, "stable");
}

#[tokio::test]
async fn add_guards() {
    let (_pg, store) = store().await;
    let (_, project) = seed_project(&store).await;

    // Authorship isn't wired yet — must fail loud, not drop silently.
    let mut authored = decision(project, "authored");
    authored.authors.push(Author::User(UserId::new()));
    assert!(matches!(
        store.decision_add(authored).await,
        Err(StoreError::Invalid(_))
    ));

    // Unknown project: FK violation surfaces as Invalid.
    let orphan = decision(ProjectId::new(), "orphan");
    assert!(matches!(
        store.decision_add(orphan).await,
        Err(StoreError::Invalid(_))
    ));
}
