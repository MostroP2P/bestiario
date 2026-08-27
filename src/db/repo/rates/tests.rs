use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::fixtures::load;

async fn migrated() -> SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

/// A snapshot with the `events` row its foreign key needs.
async fn store(pool: &SqlitePool, snapshot: &RateSnapshot) {
    events::insert_if_new(
        pool,
        &EventRecord {
            id: snapshot.event_id.clone(),
            pubkey: snapshot.pubkey.clone(),
            kind: 30078,
            created_at: snapshot.published_at,
            d_tag: Some("mostro-rates".to_string()),
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: snapshot.published_at,
        },
    )
    .await
    .expect("event");
    insert(pool, snapshot).await.expect("insert");
}

fn snapshot(id: &str, published_at: i64, usd: f64) -> RateSnapshot {
    RateSnapshot {
        event_id: id.to_string(),
        pubkey: "pk".to_string(),
        published_at,
        source: Some("yadio".to_string()),
        rates: BTreeMap::from([("USD".to_string(), usd), ("ARS".to_string(), usd * 1_000.0)]),
    }
}

#[tokio::test]
async fn a_captured_snapshot_round_trips() {
    // Arrange
    let pool = migrated().await;
    let parsed = crate::ingest::parse::rates::parse(&load(30078, "typical")).expect("parse");

    // Act
    store(&pool, &parsed).await;

    // Assert
    assert_eq!(all(&pool).await.expect("all"), vec![parsed]);
}

#[tokio::test]
async fn storing_a_snapshot_twice_leaves_one_row() {
    let pool = migrated().await;
    let snapshot = snapshot("s1", 1_000, 50_000.0);

    store(&pool, &snapshot).await;
    insert(&pool, &snapshot).await.expect("again");

    assert_eq!(all(&pool).await.expect("all").len(), 1);
}

#[tokio::test]
async fn snapshots_come_back_oldest_first_whatever_the_arrival_order() {
    let pool = migrated().await;
    store(&pool, &snapshot("late", 2_000, 51_000.0)).await;
    store(&pool, &snapshot("early", 1_000, 50_000.0)).await;

    let times: Vec<i64> = all(&pool)
        .await
        .expect("all")
        .iter()
        .map(|s| s.published_at)
        .collect();

    assert_eq!(times, vec![1_000, 2_000]);
}

#[tokio::test]
async fn clearing_empties_the_table() {
    let pool = migrated().await;
    store(&pool, &snapshot("s1", 1_000, 50_000.0)).await;

    clear(&pool).await.expect("clear");

    assert!(all(&pool).await.expect("all").is_empty());
}

/// A snapshot whose signed clock is not its `published_at` claim.
async fn store_signed(pool: &SqlitePool, id: &str, created_at: i64, published_at: i64, usd: f64) {
    let snapshot = snapshot(id, published_at, usd);
    events::insert_if_new(
        pool,
        &EventRecord {
            id: id.to_string(),
            pubkey: snapshot.pubkey.clone(),
            kind: 30078,
            created_at,
            d_tag: Some("mostro-rates".to_string()),
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: created_at,
        },
    )
    .await
    .expect("event");
    insert(pool, &snapshot).await.expect("insert");
}

#[tokio::test]
async fn the_latest_snapshot_is_the_one_nip_01_calls_current() {
    // Arrange: the event the relay would keep — the greatest `created_at` —
    // claims the *older* `published_at` of the two.
    let pool = migrated().await;
    store_signed(&pool, "older-event", 1_000, 2_000, 40_000.0).await;
    store_signed(&pool, "newer-event", 3_000, 1_500, 50_000.0).await;

    // Act
    let latest = latest_per_instance(&pool, None).await.expect("latest");

    // Assert
    assert_eq!(latest.len(), 1, "one publisher, one current version");
    assert_eq!(latest[0].event_id, "newer-event");
    assert_eq!(latest[0].published_at, 1_500);
}

#[tokio::test]
async fn two_versions_of_the_same_second_are_broken_by_the_event_id() {
    let pool = migrated().await;
    store_signed(&pool, "bbbb", 3_000, 3_000, 40_000.0).await;
    store_signed(&pool, "aaaa", 3_000, 3_000, 50_000.0).await;

    let latest = latest_per_instance(&pool, None).await.expect("latest");

    assert_eq!(latest[0].event_id, "aaaa", "the lower id wins the tie");
}

#[tokio::test]
async fn the_scope_narrows_the_query_and_not_the_answer_afterwards() {
    // Arrange: two publishers, and the unwanted one's row unreadable.
    let pool = migrated().await;
    store_signed(&pool, "mine", 3_000, 3_000, 50_000.0).await;
    let other = RateSnapshot {
        pubkey: "somebody-else".to_string(),
        ..snapshot("theirs", 3_500, 1.0)
    };
    store(&pool, &other).await;
    sqlx::query("UPDATE rates SET rates_json = ? WHERE event_id = ?")
        .bind("{not json")
        .bind("theirs")
        .execute(&pool)
        .await
        .expect("corrupt");

    // Act
    let scoped = latest_per_instance(&pool, Some("pk"))
        .await
        .expect("scoped");

    // Assert
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].event_id, "mine");
    assert!(
        latest_per_instance(&pool, None).await.is_err(),
        "unscoped, the same row is decoded and fails"
    );
}
