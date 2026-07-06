//! Users, agents, and decision authorship, against a real Postgres
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    AgentId, AgentKind, Agents, Author, DecisionFilter, DecisionStatus, Decisions, GroupKind,
    Groups, NewAgent, NewDecision, NewGroup, NewProject, NewUser, Pagination, ProjectId, Projects,
    StoreError, UserId, Users,
};
use converge_storage_postgres::PgStorage;

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

fn decision(project_id: ProjectId, authors: Vec<Author>) -> NewDecision {
    NewDecision {
        project_id,
        status: DecisionStatus::Accepted,
        title: "t".into(),
        summary: String::new(),
        context: None,
        consequences: None,
        alternatives: Vec::new(),
        authors,
        supersedes: Vec::new(),
    }
}

#[tokio::test]
async fn ensure_by_natural_key() {
    let (_pg, store) = store().await;

    let user = NewUser {
        handle: "singulared".into(),
        name: "Maksim".into(),
    };
    let first = store.user_ensure(user.clone()).await.unwrap();
    let again = store
        .user_ensure(NewUser {
            name: "Someone Else".into(),
            ..user
        })
        .await
        .unwrap();
    assert_eq!(first, again);
    // The existing row wins: display name stays as first created.
    let got = store.user_get(first).await.unwrap().unwrap();
    assert_eq!(got.handle, "singulared");
    assert_eq!(got.name, "Maksim");
    let other = store
        .user_ensure(NewUser {
            handle: "other".into(),
            name: "Other".into(),
        })
        .await
        .unwrap();
    assert_ne!(first, other);

    let claude = NewAgent {
        kind: AgentKind::Model,
        name: "claude".into(),
    };
    let model = store.agent_ensure(claude.clone()).await.unwrap();
    assert_eq!(model, store.agent_ensure(claude.clone()).await.unwrap());
    // Same name, different kind — a different agent.
    let tool = store
        .agent_ensure(NewAgent {
            kind: AgentKind::Tool,
            ..claude
        })
        .await
        .unwrap();
    assert_ne!(model, tool);
    assert_eq!(
        store.agent_get(tool).await.unwrap().unwrap().kind,
        AgentKind::Tool
    );

    assert!(store.user_get(UserId::new()).await.unwrap().is_none());
    assert!(store.agent_get(AgentId::new()).await.unwrap().is_none());

    // Lists: newest first, paged like every other resource.
    let users = store.user_list(Pagination::default()).await.unwrap();
    assert_eq!(
        users.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![other, first]
    );
    let agents = store
        .agent_list(Pagination {
            limit: Some(1),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, tool);
}

#[tokio::test]
async fn authorship_round_trip() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;
    let user = store
        .user_ensure(NewUser {
            handle: "singulared".into(),
            name: "Maksim".into(),
        })
        .await
        .unwrap();
    let agent = store
        .agent_ensure(NewAgent {
            kind: AgentKind::Model,
            name: "claude".into(),
        })
        .await
        .unwrap();

    let authors = vec![
        Author::User(user),
        Author::Agent(agent),
        Author::UserViaAgent { user, agent },
    ];
    let id = store
        .decision_add(decision(project_id, authors.clone()))
        .await
        .unwrap();

    // All three variants survive the round trip (as a set — order is
    // unspecified).
    let got = store.decision_get(id).await.unwrap().unwrap();
    assert_eq!(got.authors.len(), 3);
    for author in &authors {
        assert!(got.authors.contains(author), "missing {author:?}");
    }

    // Listing carries authorship too.
    let filter = DecisionFilter {
        project: Some(project_id),
        ..Default::default()
    };
    let listed = store
        .decision_list(filter, Pagination::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].authors.len(), 3);

    // Duplicates collapse on write: authorship is a set.
    let dup = store
        .decision_add(decision(
            project_id,
            vec![Author::User(user), Author::User(user)],
        ))
        .await
        .unwrap();
    let got = store.decision_get(dup).await.unwrap().unwrap();
    assert_eq!(got.authors, vec![Author::User(user)]);
}

#[tokio::test]
async fn unknown_author_rejected() {
    let (_pg, store) = store().await;
    let project_id = project(&store).await;

    let unknown = decision(project_id, vec![Author::User(UserId::new())]);
    assert!(matches!(
        store.decision_add(unknown).await,
        Err(StoreError::Invalid(_))
    ));

    // The insert is atomic — the failed decision left no partial row.
    let filter = DecisionFilter {
        project: Some(project_id),
        ..Default::default()
    };
    assert!(
        store
            .decision_list(filter, Pagination::default())
            .await
            .unwrap()
            .is_empty()
    );
}
