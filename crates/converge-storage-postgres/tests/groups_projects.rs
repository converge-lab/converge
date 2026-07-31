//! Round-trip tests for groups and projects, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    GroupEdit, GroupId, GroupKind, Groups, Identity, NewGroup, NewProject, Pagination, ProjectEdit,
    ProjectFilter, ProjectId, Projects, Scope, StoreError, UserId, Users,
};
use converge_storage_postgres::PgStorage;

/// A bootstrap user to own groups (pre-ACL tests run as `Scope::System`).
async fn owner(store: &PgStorage) -> UserId {
    store
        .user_login(Identity {
            provider: "local".into(),
            subject: "test".into(),
            handle: "test".into(),
            name: "Test".into(),
        })
        .await
        .unwrap()
}

fn group(name: &str, kind: GroupKind) -> NewGroup {
    NewGroup {
        name: name.into(),
        description: None,
        kind,
    }
}

#[tokio::test]
async fn group_round_trip() {
    let (_pg, store) = store().await;
    let owner = owner(&store).await;

    let id = store
        .group_add(
            owner,
            NewGroup {
                name: "platform".into(),
                description: Some("owns infra".into()),
                kind: GroupKind::Shared,
            },
        )
        .await
        .unwrap();
    let got = store.group_get(Scope::System, id).await.unwrap().unwrap();
    assert_eq!(got.id, id);
    assert_eq!(got.name, "platform");
    assert_eq!(got.description.as_deref(), Some("owns infra"));
    assert_eq!(got.kind, GroupKind::Shared);
    assert_eq!(got.owner, owner);

    let personal = store
        .group_add(owner, group("me", GroupKind::Personal))
        .await
        .unwrap();
    let all = store
        .group_list(Scope::System, Pagination::default())
        .await
        .unwrap();
    // Newest first (ULID = time order).
    assert_eq!(
        all.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![personal, id]
    );
    assert_eq!(all[0].kind, GroupKind::Personal);

    store
        .group_edit(
            Scope::System,
            id,
            vec![
                GroupEdit::SetName("platform team".into()),
                GroupEdit::SetDescription(None),
            ],
        )
        .await
        .unwrap();
    let edited = store.group_get(Scope::System, id).await.unwrap().unwrap();
    assert_eq!(edited.name, "platform team");
    assert_eq!(edited.description, None);

    assert!(
        store
            .group_get(Scope::System, GroupId::new())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store
            .group_edit(
                Scope::System,
                GroupId::new(),
                vec![GroupEdit::SetName("x".into())]
            )
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn project_round_trip() {
    let (_pg, store) = store().await;
    let owner = owner(&store).await;
    let home = store
        .group_add(owner, group("home", GroupKind::Shared))
        .await
        .unwrap();
    let other = store
        .group_add(owner, group("other", GroupKind::Shared))
        .await
        .unwrap();

    // This test asserts newest-first ordering, and same-millisecond ULIDs
    // order by their random tails — space the creations out one tick.
    let tick = || std::thread::sleep(std::time::Duration::from_millis(2));
    let p1 = store
        .project_add(
            Scope::System,
            NewProject {
                group_id: home,
                name: "api".into(),
                description: Some("the api".into()),
            },
        )
        .await
        .unwrap();
    tick();
    let p2 = store
        .project_add(
            Scope::System,
            NewProject {
                group_id: home,
                name: "web".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    tick();
    let p3 = store
        .project_add(
            Scope::System,
            NewProject {
                group_id: other,
                name: "infra".into(),
                description: None,
            },
        )
        .await
        .unwrap();

    let got = store.project_get(Scope::System, p1).await.unwrap().unwrap();
    assert_eq!(got.group_id, home);
    assert_eq!(got.name, "api");
    assert_eq!(got.description.as_deref(), Some("the api"));

    // Group filter; newest first.
    let of_home = store
        .project_list(
            Scope::System,
            ProjectFilter { group: Some(home) },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        of_home.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![p2, p1]
    );

    let latest = store
        .project_list(
            Scope::System,
            ProjectFilter::default(),
            Pagination {
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(latest.iter().map(|p| p.id).collect::<Vec<_>>(), vec![p3]);

    // Cursor paging: ids strictly older than the cursor, newest first.
    let paged = store
        .project_list(
            Scope::System,
            ProjectFilter::default(),
            Pagination {
                limit: Some(2),
                cursor: Some(p3),
            },
        )
        .await
        .unwrap();
    assert_eq!(paged.iter().map(|p| p.id).collect::<Vec<_>>(), vec![p2, p1]);

    store
        .project_edit(
            Scope::System,
            p1,
            vec![
                ProjectEdit::SetName("api-v2".into()),
                ProjectEdit::SetDescription(None),
            ],
        )
        .await
        .unwrap();
    let edited = store.project_get(Scope::System, p1).await.unwrap().unwrap();
    assert_eq!(edited.name, "api-v2");
    assert_eq!(edited.description, None);

    // Unknown group: under the ACL, missing and invisible are the same
    // answer — NotFound.
    assert!(matches!(
        store
            .project_add(
                Scope::System,
                NewProject {
                    group_id: GroupId::new(),
                    name: "orphan".into(),
                    description: None,
                }
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store
            .project_edit(
                Scope::System,
                ProjectId::new(),
                vec![ProjectEdit::SetName("x".into())]
            )
            .await,
        Err(StoreError::NotFound)
    ));
}
