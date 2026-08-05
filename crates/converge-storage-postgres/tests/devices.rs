//! Device-grant rendezvous: open, approve/deny, single-shot claim,
//! client binding, expiry (testcontainers — needs Docker).

mod common;

use common::store;
use converge_storage::{DeviceClaim, Devices, Identity, NewDeviceGrant, StoreError, Users};
use time::{Duration, OffsetDateTime};

fn grant(user_code: &str, suffix: &str, ttl: Duration) -> NewDeviceGrant {
    NewDeviceGrant {
        device_hash: format!("device-{suffix}"),
        client_hash: format!("client-{suffix}"),
        user_code: user_code.into(),
        client_name: "converge-cli @ test".into(),
        expires_at: OffsetDateTime::now_utc() + ttl,
    }
}

#[tokio::test]
async fn approve_claims_once() {
    let (_pg, store) = store().await;
    let user = store
        .user_login(Identity {
            provider: "local".into(),
            subject: "alice".into(),
            handle: "alice".into(),
            name: "Alice".into(),
        })
        .await
        .unwrap();

    store
        .device_start(grant("BCDF-GHJK", "a", Duration::minutes(15)))
        .await
        .unwrap();

    // The approval screen sees the pending grant; the client sees Pending.
    let pending = store.device_get("BCDF-GHJK").await.unwrap().unwrap();
    assert_eq!(pending.client_name, "converge-cli @ test");
    assert_eq!(
        store.device_claim("device-a", "client-a").await.unwrap(),
        DeviceClaim::Pending
    );

    // A colliding user code is a Conflict (the caller regenerates).
    assert!(matches!(
        store
            .device_start(grant("BCDF-GHJK", "other", Duration::minutes(15)))
            .await,
        Err(StoreError::Conflict(_))
    ));

    // The wrong client reads Gone and does NOT consume the grant.
    assert_eq!(
        store.device_claim("device-a", "client-b").await.unwrap(),
        DeviceClaim::Gone
    );

    store.device_decide("BCDF-GHJK", user, true).await.unwrap();
    // Decided: the approval screen no longer sees it; re-deciding is NotFound.
    assert!(store.device_get("BCDF-GHJK").await.unwrap().is_none());
    assert!(matches!(
        store.device_decide("BCDF-GHJK", user, false).await,
        Err(StoreError::NotFound)
    ));

    // The claim yields the approver exactly once.
    assert_eq!(
        store.device_claim("device-a", "client-a").await.unwrap(),
        DeviceClaim::Approved(user)
    );
    assert_eq!(
        store.device_claim("device-a", "client-a").await.unwrap(),
        DeviceClaim::Gone
    );
}

#[tokio::test]
async fn deny_and_expiry() {
    let (_pg, store) = store().await;
    let user = store
        .user_login(Identity {
            provider: "local".into(),
            subject: "bob".into(),
            handle: "bob".into(),
            name: "Bob".into(),
        })
        .await
        .unwrap();

    // Denied: reported once, then gone.
    store
        .device_start(grant("MNPQ-RSTV", "deny", Duration::minutes(15)))
        .await
        .unwrap();
    store.device_decide("MNPQ-RSTV", user, false).await.unwrap();
    assert_eq!(
        store
            .device_claim("device-deny", "client-deny")
            .await
            .unwrap(),
        DeviceClaim::Denied
    );
    assert_eq!(
        store
            .device_claim("device-deny", "client-deny")
            .await
            .unwrap(),
        DeviceClaim::Gone
    );

    // Expired: invisible to the screen, undecidable, Gone to the client.
    store
        .device_start(grant("WXZB-CDFG", "old", Duration::minutes(-1)))
        .await
        .unwrap();
    assert!(store.device_get("WXZB-CDFG").await.unwrap().is_none());
    assert!(matches!(
        store.device_decide("WXZB-CDFG", user, true).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store
            .device_claim("device-old", "client-old")
            .await
            .unwrap(),
        DeviceClaim::Gone
    );

    // Opening a new grant sweeps expired rows: the dead code is free again.
    store
        .device_start(grant("HJKL-MNPQ", "expired", Duration::minutes(-1)))
        .await
        .unwrap();
    store
        .device_start(grant("HJKL-MNPQ", "fresh", Duration::minutes(15)))
        .await
        .unwrap();
    assert!(store.device_get("HJKL-MNPQ").await.unwrap().is_some());
}
