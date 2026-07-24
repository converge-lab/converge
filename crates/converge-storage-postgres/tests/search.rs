//! Full-text search over decisions (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{
    DecisionFilter, DecisionId, DecisionStatus, Decisions, GroupKind, Groups, NewDecision,
    NewGroup, NewProject, ProjectId, Projects, StoreError,
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

async fn decision(
    store: &PgStorage,
    project_id: ProjectId,
    title: &str,
    summary: &str,
    context: Option<&str>,
) -> DecisionId {
    store
        .decision_add(NewDecision {
            project_id,
            status: DecisionStatus::Accepted,
            title: title.into(),
            summary: summary.into(),
            context: context.map(Into::into),
            consequences: None,
            alternatives: vec![],
            authors: vec![],
            supersedes: vec![],
            evidence: vec![],
        })
        .await
        .unwrap()
}

fn ids(decisions: &[converge_storage::Decision]) -> Vec<DecisionId> {
    decisions.iter().map(|d| d.id).collect()
}

#[tokio::test]
async fn ranked_stemmed_and_filtered() {
    let (_pg, store) = store().await;
    let p1 = project(&store).await;
    let p2 = project(&store).await;

    // The same topic at three weights: title > summary > context.
    let in_title = decision(&store, p1, "Cache invalidation strategy", "", None).await;
    let in_summary = decision(&store, p1, "Session storage", "we cache the index", None).await;
    let in_context = decision(
        &store,
        p2,
        "Deployment shape",
        "one binary",
        Some("the cache layer stays external"),
    )
    .await;
    decision(&store, p1, "Unrelated topic", "nothing here", None).await;

    // Stemmed ("caching" ~ "cache"), weighted: title first, context last.
    let hits = store
        .decision_search("caching", DecisionFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(ids(&hits), vec![in_title, in_summary, in_context]);

    // The filter composes: p2 narrows to the context hit.
    let hits = store
        .decision_search(
            "caching",
            DecisionFilter {
                project: Some(p2),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(ids(&hits), vec![in_context]);

    // Websearch syntax: exclusion and quoted phrases.
    let hits = store
        .decision_search("cache -invalidation", DecisionFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(ids(&hits), vec![in_summary, in_context]);
    let hits = store
        .decision_search("\"cache invalidation\"", DecisionFilter::default(), None)
        .await
        .unwrap();
    assert_eq!(ids(&hits), vec![in_title]);

    // The limit caps ranked results from the top.
    let hits = store
        .decision_search("caching", DecisionFilter::default(), Some(1))
        .await
        .unwrap();
    assert_eq!(ids(&hits), vec![in_title]);

    // No terms is an error, not an empty page; no *matches* is fine.
    assert!(matches!(
        store
            .decision_search("  ", DecisionFilter::default(), None)
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .decision_search("-", DecisionFilter::default(), None)
            .await,
        Err(StoreError::Invalid(_))
    ));
    let hits = store
        .decision_search("zeppelin", DecisionFilter::default(), None)
        .await
        .unwrap();
    assert!(hits.is_empty());
}
