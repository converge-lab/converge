//! Round-trip tests for groups and projects, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    GroupEdit, GroupId, GroupKind, Groups, NewGroup, NewProject, ProjectEdit, ProjectFilter,
    ProjectId, Projects, StoreError,
};

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

    let id = store
        .group_add(NewGroup {
            name: "platform".into(),
            description: Some("owns infra".into()),
            kind: GroupKind::Shared,
        })
        .await
        .unwrap();
    let got = store.group_get(id).await.unwrap().unwrap();
    assert_eq!(got.id, id);
    assert_eq!(got.name, "platform");
    assert_eq!(got.description.as_deref(), Some("owns infra"));
    assert_eq!(got.kind, GroupKind::Shared);

    let personal = store
        .group_add(group("me", GroupKind::Personal))
        .await
        .unwrap();
    let all = store.group_list().await.unwrap();
    // Newest first (ULID = time order).
    assert_eq!(
        all.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![personal, id]
    );
    assert_eq!(all[0].kind, GroupKind::Personal);

    store
        .group_edit(
            id,
            vec![
                GroupEdit::SetName("platform team".into()),
                GroupEdit::SetDescription(None),
            ],
        )
        .await
        .unwrap();
    let edited = store.group_get(id).await.unwrap().unwrap();
    assert_eq!(edited.name, "platform team");
    assert_eq!(edited.description, None);

    assert!(store.group_get(GroupId::new()).await.unwrap().is_none());
    assert!(matches!(
        store
            .group_edit(GroupId::new(), vec![GroupEdit::SetName("x".into())])
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn project_round_trip() {
    let (_pg, store) = store().await;
    let home = store
        .group_add(group("home", GroupKind::Shared))
        .await
        .unwrap();
    let other = store
        .group_add(group("other", GroupKind::Shared))
        .await
        .unwrap();

    let p1 = store
        .project_add(NewProject {
            group_id: home,
            name: "api".into(),
            description: Some("the api".into()),
        })
        .await
        .unwrap();
    let p2 = store
        .project_add(NewProject {
            group_id: home,
            name: "web".into(),
            description: None,
        })
        .await
        .unwrap();
    let p3 = store
        .project_add(NewProject {
            group_id: other,
            name: "infra".into(),
            description: None,
        })
        .await
        .unwrap();

    let got = store.project_get(p1).await.unwrap().unwrap();
    assert_eq!(got.group_id, home);
    assert_eq!(got.name, "api");
    assert_eq!(got.description.as_deref(), Some("the api"));

    // Group filter; newest first.
    let of_home = store
        .project_list(ProjectFilter {
            group: Some(home),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        of_home.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![p2, p1]
    );

    let latest = store
        .project_list(ProjectFilter {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(latest.iter().map(|p| p.id).collect::<Vec<_>>(), vec![p3]);

    store
        .project_edit(
            p1,
            vec![
                ProjectEdit::SetName("api-v2".into()),
                ProjectEdit::SetDescription(None),
            ],
        )
        .await
        .unwrap();
    let edited = store.project_get(p1).await.unwrap().unwrap();
    assert_eq!(edited.name, "api-v2");
    assert_eq!(edited.description, None);

    // Unknown group: FK violation surfaces as Invalid.
    assert!(matches!(
        store
            .project_add(NewProject {
                group_id: GroupId::new(),
                name: "orphan".into(),
                description: None,
            })
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .project_edit(ProjectId::new(), vec![ProjectEdit::SetName("x".into())])
            .await,
        Err(StoreError::NotFound)
    ));
}
