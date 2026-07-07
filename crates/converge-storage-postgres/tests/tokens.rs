//! Token lifecycle: mint, authenticate, list, revoke, owner scoping
//! (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{Identity, Pagination, StoreError, TokenId, Tokens, Users};

fn local(handle: &str) -> Identity {
    Identity {
        provider: "local".into(),
        subject: handle.into(),
        handle: handle.into(),
        name: handle.into(),
    }
}

#[tokio::test]
async fn lifecycle_and_owner_scoping() {
    let (_pg, store) = store().await;
    let alice = store.user_login(local("alice")).await.unwrap();
    let bob = store.user_login(local("bob")).await.unwrap();

    let id = store
        .token_add(alice, "laptop".into(), "hash-a".into())
        .await
        .unwrap();
    store
        .token_add(bob, "ci".into(), "hash-b".into())
        .await
        .unwrap();

    // The authentication lookup resolves to the owner.
    assert_eq!(store.token_user("hash-a").await.unwrap(), Some(alice));
    assert_eq!(store.token_user("hash-nope").await.unwrap(), None);

    // Lists are per-owner.
    let mine = store
        .token_list(alice, Pagination::default())
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, id);
    assert_eq!(mine[0].label, "laptop");

    // Bob cannot revoke Alice's token — it reads as absent…
    assert!(matches!(
        store.token_revoke(bob, id).await,
        Err(StoreError::NotFound)
    ));
    // …and the credential still works.
    assert_eq!(store.token_user("hash-a").await.unwrap(), Some(alice));

    // The owner can; the credential dies with the row.
    store.token_revoke(alice, id).await.unwrap();
    assert_eq!(store.token_user("hash-a").await.unwrap(), None);
    assert!(matches!(
        store.token_revoke(alice, id).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.token_revoke(alice, TokenId::new()).await,
        Err(StoreError::NotFound)
    ));
}
