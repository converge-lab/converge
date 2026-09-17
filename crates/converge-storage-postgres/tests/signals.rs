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
    user_named(store, "tester").await
}

async fn user_named(store: &PgStorage, subject: &str) -> UserId {
    store
        .user_login(Identity {
            provider: "local".into(),
            subject: subject.into(),
            handle: subject.into(),
            name: subject.into(),
        })
        .await
        .unwrap()
}

async fn decision(store: &PgStorage, project_id: ProjectId, title: &str) -> DecisionId {
    let by = user_named(store, "author").await;
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
                authors: vec![Author::User(by)],
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

#[tokio::test]
async fn since_pages_forward_oldest_first() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;
    let c = decision(&store, p, "c").await;
    let d = decision(&store, p, "d").await;
    let mut ids = vec![];
    for target in [b, c, d] {
        ids.push(
            store
                .signal_add(
                    Scope::System,
                    signal(a, vec![target], "dependency", Tier::Watch, me),
                )
                .await
                .unwrap(),
        );
    }
    ids.sort_unstable(); // ascending by id (creation order only to the millisecond)
    let [lo, mid, hi] = ids[..] else {
        unreachable!()
    };
    let list = |filter: SignalFilter, page: Pagination<SignalId>| {
        let store = store.clone();
        async move {
            store
                .signal_list(Scope::System, filter, page)
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        }
    };
    let after_lo = SignalFilter {
        since: Some(lo),
        ..Default::default()
    };

    // Everything after `lo`, oldest first.
    assert_eq!(
        list(after_lo.clone(), Pagination::default()).await,
        vec![mid, hi]
    );
    // A limit truncates the newest and never skips the oldest — the
    // property a reader catching up relies on.
    assert_eq!(
        list(
            after_lo.clone(),
            Pagination {
                limit: Some(1),
                cursor: None
            }
        )
        .await,
        vec![mid]
    );
    // Without `since`, a listing: newest first.
    assert_eq!(
        list(SignalFilter::default(), Pagination::default()).await,
        vec![hi, mid, lo]
    );
    // Paging both ways at once is a contradiction, not a query.
    assert!(matches!(
        store
            .signal_list(
                Scope::System,
                after_lo,
                Pagination {
                    limit: None,
                    cursor: Some(hi)
                }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn claim_hands_out_what_arrived_after_the_session_began_once() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;
    let c = decision(&store, p, "c").await;
    let d = decision(&store, p, "d").await;
    let e = decision(&store, p, "e").await;
    let claim = |session: &'static str, limit: u32| {
        let store = store.clone();
        async move {
            store
                .signal_claim(Scope::User(me), session, Some("codex"), limit)
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        }
    };
    let add = |source: DecisionId, target: DecisionId, kind: &'static str| {
        let store = store.clone();
        async move {
            store
                .signal_add(
                    Scope::System,
                    signal(source, vec![target], kind, Tier::Coordinate, me),
                )
                .await
                .unwrap()
        }
    };

    // Before any session: a signal that is the listing's job, never a
    // claim's.
    let before = add(a, b, "dependency").await;

    // A session-start hook draws the line by receipting what it listed.
    store
        .signal_receive(Scope::User(me), "s1", Some("codex"), &[before])
        .await
        .unwrap();
    assert_eq!(claim("s1", 10).await, vec![]);

    // What is recorded after arrives oldest first, a page at a time, once.
    let mut ids = vec![add(a, c, "dependency").await, add(a, d, "dependency").await];
    ids.push(add(a, e, "dependency").await);
    ids.sort_unstable();
    assert_eq!(claim("s1", 2).await, ids[..2]);
    assert_eq!(claim("s1", 10).await, ids[2..]);
    assert_eq!(claim("s1", 10).await, vec![]);

    // A session first seen by a claim is created then and gets nothing;
    // from there each session is handed a signal independently.
    assert_eq!(claim("s2", 10).await, vec![]);
    let late = add(b, c, "duplication").await;
    assert_eq!(claim("s2", 10).await, vec![late]);
    assert_eq!(claim("s1", 10).await, vec![late]);

    // Shown at a session start after the line was drawn: not handed again.
    let shown = add(b, d, "divergence").await;
    store
        .signal_receive(Scope::User(me), "s1", None, &[shown])
        .await
        .unwrap();
    assert_eq!(claim("s1", 10).await, vec![]);

    // A signal already judged is not a delivery.
    let judged = add(c, d, "duplication").await;
    store
        .signal_resolve(
            Scope::System,
            judged,
            SignalStatus::Dismissed,
            Author::User(me),
        )
        .await
        .unwrap();
    assert_eq!(claim("s1", 10).await, vec![]);

    // System has no sessions; a blank session is no session.
    assert!(matches!(
        store.signal_claim(Scope::System, "s1", None, 1).await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store.signal_claim(Scope::User(me), "  ", None, 1).await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn unseen_means_no_receipt_in_any_session() {
    let (_pg, store) = store().await;
    let me = user(&store).await;
    let p = project(&store, me).await;
    let a = decision(&store, p, "a").await;
    let b = decision(&store, p, "b").await;
    let c = decision(&store, p, "c").await;
    let x = store
        .signal_add(
            Scope::System,
            signal(a, vec![b], "dependency", Tier::Watch, me),
        )
        .await
        .unwrap();
    let y = store
        .signal_add(
            Scope::System,
            signal(a, vec![c], "dependency", Tier::Watch, me),
        )
        .await
        .unwrap();
    let unseen = |scope: Scope| {
        let store = store.clone();
        async move {
            let mut ids: Vec<_> = store
                .signal_list(
                    scope,
                    SignalFilter {
                        unseen: true,
                        ..Default::default()
                    },
                    Pagination::default(),
                )
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect();
            ids.sort_unstable();
            ids
        }
    };
    let mut both = vec![x, y];
    both.sort_unstable();
    assert_eq!(unseen(Scope::User(me)).await, both);

    // Read on the web (`session = ""`): seen, wherever else it goes.
    store
        .signal_receive(Scope::User(me), "", None, &[x])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(me)).await, vec![y]);
    // Handed to a harness session: seen too — a receipt is a receipt.
    store
        .signal_receive(Scope::User(me), "s1", Some("claude"), &[y])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(me)).await, vec![]);
    // System has no receipts, so the filter is a no-op there.
    assert_eq!(unseen(Scope::System).await, both);
    // A receipt is idempotent, and one for an invisible signal is ignored.
    store
        .signal_receive(Scope::User(me), "", None, &[x, x])
        .await
        .unwrap();
    let stranger = user_named(&store, "stranger").await;
    store
        .signal_receive(Scope::User(stranger), "", None, &[x])
        .await
        .unwrap();
    assert_eq!(unseen(Scope::User(stranger)).await, vec![]);
    assert!(matches!(
        store.signal_receive(Scope::System, "", None, &[x]).await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn claim_never_crosses_groups() {
    let (_pg, store) = store().await;
    let alice = user_named(&store, "alice").await;
    let bob = user_named(&store, "bob").await;
    let _theirs = project(&store, alice).await; // alice's own group
    let bobs = project(&store, bob).await; // bob's — alice is no member
    for (who, session) in [(alice, "a1"), (bob, "b1")] {
        store
            .signal_receive(Scope::User(who), session, Some("claude"), &[])
            .await
            .unwrap();
    }
    let x = decision(&store, bobs, "x").await;
    let y = decision(&store, bobs, "y").await;
    let conflict = store
        .signal_add(
            Scope::System,
            signal(x, vec![y], "dependency", Tier::Conflict, bob),
        )
        .await
        .unwrap();

    // The ledger reads through the same group gate as every listing:
    // bob's conflict reaches bob's session and never alice's.
    let ids = |signals: Vec<converge_storage::Signal>| {
        signals.into_iter().map(|s| s.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(store
            .signal_claim(Scope::User(alice), "a1", None, 10)
            .await
            .unwrap()),
        vec![]
    );
    assert_eq!(
        ids(store
            .signal_claim(Scope::User(bob), "b1", None, 10)
            .await
            .unwrap()),
        vec![conflict]
    );
}
