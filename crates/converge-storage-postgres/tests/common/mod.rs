//! Shared test harness: a fresh Postgres per test (testcontainers — needs
//! Docker).

use converge_storage_postgres::PgStorage;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

/// Expected list order: the seeded ids, newest first. The storage
/// contract is `order by id desc` — a total order that tracks creation
/// only down to the ULID's millisecond (same-tick ids order by their
/// random tails) — so ordered expectations are COMPUTED from the ids,
/// never hand-written from creation order.
#[allow(dead_code)] // not every suite asserts ordering
pub fn newest_first<T: Ord + Copy>(ids: &[T]) -> Vec<T> {
    let mut ids = ids.to_vec();
    ids.sort_unstable_by_key(|&id| std::cmp::Reverse(id));
    ids
}

/// Boot a fresh Postgres, migrate, connect. The container lives as long as
/// the returned handle.
pub async fn store() -> (ContainerAsync<Postgres>, PgStorage) {
    let node = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start postgres (is Docker running?)");
    let port = node.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = PgStorage::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    (node, store)
}
