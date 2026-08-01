//! Signals: typed decision → decisions observations (testcontainers —
//! needs Docker).

mod common;

use common::{newest_first, store};
use converge_storage::{
    Author, DecisionId, DecisionStatus, Decisions, GroupId, GroupKind, Groups, Identity,
    NewDecision, NewGroup, NewProject, NewSignal, Pagination, ProjectId, Projects, Scope,
    SignalFilter, SignalId, SignalStatus, Signals, StoreError, Tier, UserId, Users,
};
use converge_storage_postgres::PgStorage;

async fn group(store: &PgStorage, owner: UserId) -> GroupId {
    store
        .group_add(
            owner,
            NewGroup {
                name: "g".into(),
                description: None,
                kind: GroupKind::Shared,
            },
        )
        .await
        .unwrap()
}

async fn project_in(store: &PgStorage, group: GroupId) -> ProjectId {
    store
        .project_add(
            Scope::System,
            NewProject {
                group_id: group,
                name: "p".into(),
                description: None,
            },
        )
        .await
        .unwrap()
}

async fn project(store: &PgStorage, owner: UserId) -> ProjectId {
    let group = group(store, owner).await;
    project_in(store, group).await
}

async fn user(store: &PgStorage) -> UserId {
    store
        .user_login(Identity {
            provider: "local".into(),
            subject: "tester".into(),
            handle: "tester".into(),
            name: "Tester".into(),
        })
        .await
        .unwrap()
}

async fn decision(store: &PgStorage, project_id: ProjectId, title: &str) -> DecisionId {
    store
        .decision_add(
            Scope::System,
            NewDecision {
                project_id,
                status: DecisionStatus::Accepted,
                title: title.into(),
                summary: String::new(),
                context: None,
                consequences: None,
                alternatives: vec![],
                authors: vec![],
                supersedes: vec![],
                evidence: vec![],
            },
        )
        .await
        .unwrap()
}

fn signal(
    source: DecisionId,
    targets: Vec<DecisionId>,
    kind: &str,
    tier: Tier,
    by: UserId,
) -> NewSignal {
    NewSignal {
        source,
        targets,
        kind: kind.into(),
        tier,
        title: "one solved next door".into(),
        text: "the neighbor already did this".into(),
        consequence: Some("duplicate effort".into()),
        recommendation: Some("reuse theirs".into()),
        produced_by: Author::User(by),
    }
}

#[tokio::test]
async fn round_trip_and_invariants() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;
    let c = decision(&store, p, "c").await;

    // Targets collapse to a set; the record comes back whole.
    let id = store
        .signal_add(
            Scope::System,
            signal(a, vec![b, c, b], "duplication", Tier::Watch, me),
        )
        .await
        .unwrap();
    let got = store.signal_get(Scope::System, id).await.unwrap().unwrap();
    assert_eq!(got.source, a);
    // Stable but unspecified order (same-millisecond ULIDs don't sort
    // by creation) — compare as a set.
    let mut want = vec![b, c];
    want.sort_unstable();
    assert_eq!(got.targets, want);
    assert_eq!(got.kind, "duplication");
    assert_eq!(got.tier, Tier::Watch);
    assert_eq!(got.status, SignalStatus::Proposed);
    assert_eq!(got.produced_by, Author::User(me));
    assert_eq!(got.resolved_by, None);
    assert_eq!(got.consequence.as_deref(), Some("duplicate effort"));

    // The invariants fail loudly.
    for (new, needle) in [
        (signal(a, vec![], "x", Tier::Watch, me), "target"),
        (signal(a, vec![a, b], "x", Tier::Watch, me), "own source"),
        (signal(a, vec![b], "  ", Tier::Watch, me), "kind"),
    ] {
        match store.signal_add(Scope::System, new).await {
            Err(StoreError::Invalid(m)) => assert!(m.contains(needle), "{m} ∌ {needle}"),
            other => panic!("expected Invalid({needle}), got {other:?}"),
        }
    }

    // Unknown decisions are caught by the references.
    let ghost = DecisionId::new();
    assert!(matches!(
        store
            .signal_add(Scope::System, signal(ghost, vec![b], "x", Tier::Watch, me))
            .await,
        Err(StoreError::Invalid(_))
    ));

    // Unknown signal id reads as absent.
    assert_eq!(
        store
            .signal_get(Scope::System, SignalId::new())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn pairs_are_never_re_raised() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;
    let c = decision(&store, p, "c").await;

    let first = store
        .signal_add(
            Scope::System,
            signal(a, vec![b], "duplication", Tier::Watch, me),
        )
        .await
        .unwrap();

    // Any overlap with a recorded (source, target, kind) pair conflicts —
    // including via a wider target set.
    assert!(matches!(
        store
            .signal_add(
                Scope::System,
                signal(a, vec![b, c], "duplication", Tier::Watch, me)
            )
            .await,
        Err(StoreError::Conflict(_))
    ));

    // A different kind is a different relationship.
    store
        .signal_add(
            Scope::System,
            signal(a, vec![b], "dependency", Tier::Coordinate, me),
        )
        .await
        .unwrap();

    // Dismissal does not reopen the pair: dismissed observations are the
    // don't-re-raise memory.
    store
        .signal_resolve(
            Scope::System,
            first,
            SignalStatus::Dismissed,
            Author::User(me),
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .signal_add(
                Scope::System,
                signal(a, vec![b], "duplication", Tier::Watch, me)
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
}

#[tokio::test]
async fn resolution_stamps_the_judge() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;

    let id = store
        .signal_add(
            Scope::System,
            signal(a, vec![b], "dependency", Tier::Conflict, me),
        )
        .await
        .unwrap();

    // `proposed` is not a resolution.
    assert!(matches!(
        store
            .signal_resolve(Scope::System, id, SignalStatus::Proposed, Author::User(me))
            .await,
        Err(StoreError::Invalid(_))
    ));

    store
        .signal_resolve(Scope::System, id, SignalStatus::Confirmed, Author::User(me))
        .await
        .unwrap();
    let got = store.signal_get(Scope::System, id).await.unwrap().unwrap();
    assert_eq!(got.status, SignalStatus::Confirmed);
    assert_eq!(got.resolved_by, Some(Author::User(me)));

    // Re-resolving flips the verdict (and would restamp the judge).
    store
        .signal_resolve(Scope::System, id, SignalStatus::Dismissed, Author::User(me))
        .await
        .unwrap();
    let got = store.signal_get(Scope::System, id).await.unwrap().unwrap();
    assert_eq!(got.status, SignalStatus::Dismissed);

    // Unknown signals are NotFound.
    assert!(matches!(
        store
            .signal_resolve(
                Scope::System,
                SignalId::new(),
                SignalStatus::Confirmed,
                Author::User(me)
            )
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn list_filters_match_either_end() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    // One group: signals never cross groups, and this test is about the
    // either-end filters, not the boundary.
    let g = group(&store, me).await;
    let p1 = project_in(&store, g).await;
    let p2 = project_in(&store, g).await;
    let a = decision(&store, p1, "a").await; // p1
    let b = decision(&store, p2, "b").await; // p2
    let c = decision(&store, p2, "c").await; // p2

    let s1 = store
        .signal_add(
            Scope::System,
            signal(a, vec![b], "dependency", Tier::Conflict, me),
        )
        .await
        .unwrap();
    let s2 = store
        .signal_add(
            Scope::System,
            signal(c, vec![b], "duplication", Tier::Watch, me),
        )
        .await
        .unwrap();

    let list = |filter: SignalFilter| {
        let store = store.clone();
        async move {
            store
                .signal_list(Scope::System, filter, Pagination::default())
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        }
    };

    // Project matches either end: p1 only touches s1 (via its source);
    // p2 touches both (b is a target of both).
    assert_eq!(
        list(SignalFilter {
            project: Some(p1),
            ..Default::default()
        })
        .await,
        vec![s1]
    );
    assert_eq!(
        list(SignalFilter {
            project: Some(p2),
            ..Default::default()
        })
        .await,
        newest_first(&[s1, s2]) // id-descending (computed — see common)
    );

    // Decision matches either end.
    assert_eq!(
        list(SignalFilter {
            decision: Some(b),
            ..Default::default()
        })
        .await,
        newest_first(&[s1, s2])
    );
    assert_eq!(
        list(SignalFilter {
            decision: Some(a),
            ..Default::default()
        })
        .await,
        vec![s1]
    );

    // Tier and status narrow.
    assert_eq!(
        list(SignalFilter {
            tier: Some(Tier::Watch),
            ..Default::default()
        })
        .await,
        vec![s2]
    );
    store
        .signal_resolve(Scope::System, s1, SignalStatus::Dismissed, Author::User(me))
        .await
        .unwrap();
    assert_eq!(
        list(SignalFilter {
            status: Some(SignalStatus::Dismissed),
            ..Default::default()
        })
        .await,
        vec![s1]
    );
}
