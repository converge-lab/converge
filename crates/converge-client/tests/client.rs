//! The client against the real server over a real socket
//! (testcontainers — needs Docker).

use converge_client::Client;
use converge_server::app;
use converge_server::auth;
use converge_server::auth::Sessions;
use converge_storage::{
    DecisionEdit, DecisionFilter, DecisionId, DecisionStatus, GroupKind, Identity, NewDecision,
    NewGroup, NewProject, Pagination, StoreError,
};
use converge_storage::{Tokens, Users};
use converge_storage_postgres::PgStorage;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tokio::net::TcpListener;

/// Boot Postgres + the server on an ephemeral port; aim a client at it.
async fn client() -> (ContainerAsync<Postgres>, Client) {
    let node = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start postgres (is Docker running?)");
    let port = node.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStorage::connect(&url).await.unwrap();
    store.migrate().await.unwrap();

    let me = Identity {
        provider: "local".into(),
        subject: "admin".into(),
        handle: "admin".into(),
        name: "Admin".into(),
    };
    let admin = store.user_login(me.clone()).await.unwrap();
    store
        .token_add(admin, "test".into(), auth::hash("cvg_test"))
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(
                store,
                me,
                Sessions::new(Some("test-session-secret")),
                None,
                None,
            ),
        )
        .await
        .unwrap();
    });
    let client = Client::new(format!("http://{addr}").parse().unwrap()).with_token("cvg_test");
    (node, client)
}

fn decision(project_id: converge_storage::ProjectId, title: &str) -> NewDecision {
    NewDecision {
        project_id,
        status: DecisionStatus::Accepted,
        title: title.into(),
        summary: String::new(),
        context: None,
        consequences: None,
        alternatives: Vec::new(),
        authors: Vec::new(),
        supersedes: Vec::new(),
    }
}

#[tokio::test]
async fn round_trip() {
    let (_pg, api) = client().await;

    // Identity resolves and is stable.
    let me = api.me().await.unwrap();
    assert_eq!(me.handle, "admin");
    assert_eq!(api.me().await.unwrap().id, me.id);

    // Token lifecycle: mint → the secret authenticates → revoke kills it.
    let minted = api
        .token_add(&converge_client::NewToken {
            label: "laptop".into(),
        })
        .await
        .unwrap();
    let fresh = Client::new(api.base().clone()).with_token(minted.token.clone());
    assert_eq!(fresh.me().await.unwrap().id, me.id);
    let tokens = api.token_list(&Pagination::default()).await.unwrap();
    assert!(tokens.items.iter().any(|t| t.id == minted.id));
    api.token_revoke(minted.id).await.unwrap();
    assert!(matches!(fresh.me().await, Err(StoreError::Unauthorized)));
    assert!(matches!(
        api.token_revoke(minted.id).await,
        Err(StoreError::NotFound)
    ));
    let users = api.user_list(&Pagination::default()).await.unwrap();
    assert!(users.items.iter().any(|u| u.id == me.id));
    assert!(
        api.agent_list(&Pagination::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );

    // Groups + projects through the typed surface.
    let group = api
        .group_add(&NewGroup {
            name: "platform".into(),
            description: None,
            kind: GroupKind::Shared,
        })
        .await
        .unwrap();
    let project = api
        .project_add(&NewProject {
            group_id: group,
            name: "converge".into(),
            description: None,
        })
        .await
        .unwrap();
    assert_eq!(
        api.project_get(project).await.unwrap().unwrap().group_id,
        group
    );
    let projected = api
        .group_projects(group, &Pagination::default())
        .await
        .unwrap();
    assert_eq!(projected.items.len(), 1);

    // Decisions: create, supersede at birth, read the derived status back.
    let a = api.decision_add(&decision(project, "A")).await.unwrap();
    let b = api
        .decision_add(&NewDecision {
            supersedes: vec![a],
            ..decision(project, "B")
        })
        .await
        .unwrap();
    assert_eq!(
        api.decision_get(a).await.unwrap().unwrap().status,
        DecisionStatus::Superseded
    );
    let edges = api.decision_edges(a).await.unwrap().unwrap();
    assert_eq!(edges.superseded_by, vec![b]);

    // The edit batch, then the feed sees the change.
    api.decision_edit(b, &[DecisionEdit::SetContext(Some("ctx".into()))])
        .await
        .unwrap();
    let feed = api
        .group_decisions(group, &DecisionFilter::default(), &Pagination::default())
        .await
        .unwrap();
    assert_eq!(feed.items.len(), 2);
    assert_eq!(feed.items[0].context.as_deref(), Some("ctx"));

    // Cursor walk: no overlap, no loss, exhaustion.
    let first = api
        .decision_list(
            &DecisionFilter::default(),
            &Pagination {
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    let cursor: DecisionId = first
        .next_cursor
        .clone()
        .unwrap()
        .parse::<ulid::Ulid>()
        .unwrap()
        .into();
    let rest = api
        .decision_list(
            &DecisionFilter::default(),
            &Pagination {
                limit: Some(2),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert_ne!(rest.items[0].id, first.items[0].id);
    assert!(rest.next_cursor.is_none());
}

#[tokio::test]
async fn errors_map_back_to_the_domain() {
    let (_pg, api) = client().await;

    // Missing resources: get is None, edit is NotFound.
    assert!(api.decision_get(DecisionId::new()).await.unwrap().is_none());
    assert!(
        api.decision_edges(DecisionId::new())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        api.decision_edit(DecisionId::new(), &[DecisionEdit::SetTitle("x".into())])
            .await,
        Err(StoreError::NotFound)
    ));

    // Domain guards surface as Invalid with the server's message.
    let group = api
        .group_add(&NewGroup {
            name: "g".into(),
            description: None,
            kind: GroupKind::Shared,
        })
        .await
        .unwrap();
    let project = api
        .project_add(&NewProject {
            group_id: group,
            name: "p".into(),
            description: None,
        })
        .await
        .unwrap();
    let err = api
        .decision_add(&NewDecision {
            status: DecisionStatus::Superseded,
            ..decision(project, "x")
        })
        .await
        .unwrap_err();
    match err {
        StoreError::Invalid(m) => assert!(m.contains("derived")),
        other => panic!("expected Invalid, got {other:?}"),
    }

    // A dead server reads as Unavailable.
    let dead = Client::new("http://127.0.0.1:9".parse().unwrap());
    assert!(matches!(dead.me().await, Err(StoreError::Unavailable(_))));
}
