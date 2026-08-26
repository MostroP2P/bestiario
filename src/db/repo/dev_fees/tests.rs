//! Dev fees and the duplicate flag, against a real migrated SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::fixtures::load;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const PUBKEY: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const ORDER: &str = "0f4f0a1e-9c22-4b7e-9c19-2a6f4b1d2e30";
const OTHER_ORDER: &str = "9c1b1f7a-3d55-4a6e-8f21-0b7c5e2d4a91";
/// Three publications, ten minutes apart in the instance's clock.
const T0: i64 = 1_787_700_000;
const T1: i64 = 1_787_700_600;
const T2: i64 = 1_787_701_200;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

/// A fee for `order_id`, published at `created_at`.
fn fee(order_id: &str, created_at: i64) -> DevFee {
    DevFee {
        event_id: format!("{created_at:064x}"),
        pubkey: PUBKEY.to_string(),
        order_id: order_id.to_string(),
        created_at,
        amount_sats: 126,
        payment_hash: format!("hash-{created_at}"),
        destination: Some("mostrop2p@getalby.com".to_string()),
        network: Some(crate::network::Network::Mainnet),
    }
}

/// Stores the `events` row the foreign key needs, then the fee.
async fn ingest(pool: &SqlitePool, fee: &DevFee) {
    let record = EventRecord {
        id: fee.event_id.clone(),
        pubkey: fee.pubkey.clone(),
        kind: 8383,
        created_at: fee.created_at,
        d_tag: None,
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: fee.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    insert(pool, fee).await.expect("fee");
}

/// The stored fees for an order, oldest first, as `(event_id, is_duplicate)`.
async fn flags(pool: &SqlitePool, order_id: &str) -> Vec<(String, bool)> {
    for_order(pool, order_id)
        .await
        .expect("read")
        .into_iter()
        .map(|stored| (stored.fee.event_id, stored.is_duplicate))
        .collect()
}

#[tokio::test]
async fn a_captured_fee_round_trips() {
    // Arrange
    let pool = migrated().await;
    let parsed = crate::ingest::parse::dev_fee::parse(&load(8383, "typical")).expect("parse");

    // Act
    ingest(&pool, &parsed).await;

    // Assert
    let stored = for_order(&pool, &parsed.order_id).await.expect("read");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].fee, parsed);
    assert!(!stored[0].is_duplicate);
}

#[tokio::test]
async fn an_orphan_fee_inserts_cleanly() {
    // Relay retention for dev fees is a year against a fortnight for orders
    // (SPEC 2.2), so a fee naming an order nobody has seen is the normal case
    // during backfill, not a broken row. There is no foreign key to violate.
    let pool = migrated().await;

    ingest(&pool, &fee(ORDER, T0)).await;

    let stored = for_order(&pool, ORDER).await.expect("read");
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].is_duplicate);
}

#[tokio::test]
async fn a_second_fee_for_one_order_is_flagged_and_the_first_stays_canonical() {
    // mostrod bug #620 pays the dev fee twice for the same order. Both events
    // are real and both are kept, but only one of them is a settlement:
    // counting the second would inflate dev-fee volume.
    let pool = migrated().await;

    ingest(&pool, &fee(ORDER, T0)).await;
    ingest(&pool, &fee(ORDER, T1)).await;

    assert_eq!(
        flags(&pool, ORDER).await,
        vec![(format!("{T0:064x}"), false), (format!("{T1:064x}"), true),]
    );
}

#[tokio::test]
async fn the_earliest_fee_is_canonical_however_it_arrives() {
    // Backfill walks backwards, so the duplicate lands first. The flags are
    // recomputed for the whole order rather than decided on arrival, which is
    // what makes the two orders agree.
    let pool = migrated().await;

    ingest(&pool, &fee(ORDER, T2)).await;
    ingest(&pool, &fee(ORDER, T0)).await;
    ingest(&pool, &fee(ORDER, T1)).await;

    assert_eq!(
        flags(&pool, ORDER).await,
        vec![
            (format!("{T0:064x}"), false),
            (format!("{T1:064x}"), true),
            (format!("{T2:064x}"), true),
        ]
    );
}

#[tokio::test]
async fn fees_for_different_orders_are_not_duplicates_of_each_other() {
    let pool = migrated().await;

    ingest(&pool, &fee(ORDER, T0)).await;
    ingest(&pool, &fee(OTHER_ORDER, T1)).await;

    assert_eq!(
        flags(&pool, ORDER).await,
        vec![(format!("{T0:064x}"), false)]
    );
    assert_eq!(
        flags(&pool, OTHER_ORDER).await,
        vec![(format!("{T1:064x}"), false)]
    );
}

#[tokio::test]
async fn the_same_fee_twice_leaves_one_row_and_no_duplicate_flag() {
    // An event delivered by three relays is one payment, not three.
    let pool = migrated().await;
    let fee = fee(ORDER, T0);

    ingest(&pool, &fee).await;
    insert(&pool, &fee).await.expect("replay");

    assert_eq!(flags(&pool, ORDER).await, vec![(fee.event_id, false)]);
}

#[tokio::test]
async fn a_fee_can_be_stored_inside_the_pipeline_transaction() {
    // SPEC 8.1 step 7 persists the version and its projection in one
    // transaction, so the executor the pipeline hands down is a `&mut`
    // reference rather than a pool. Both statements of `insert` have to run on
    // it, and a rollback has to take the fee with it.
    let pool = migrated().await;
    let fee = fee(ORDER, T0);
    let record = EventRecord {
        id: fee.event_id.clone(),
        pubkey: fee.pubkey.clone(),
        kind: 8383,
        created_at: fee.created_at,
        d_tag: None,
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: fee.created_at,
    };

    let mut transaction = pool.begin().await.expect("begin");
    events::insert_if_new(&mut *transaction, &record)
        .await
        .expect("event");
    insert(&mut *transaction, &fee).await.expect("fee");
    transaction.rollback().await.expect("rollback");

    assert!(for_order(&pool, ORDER).await.expect("read").is_empty());
}

#[tokio::test]
async fn a_committed_transaction_leaves_the_fee_flagged_correctly() {
    let pool = migrated().await;
    ingest(&pool, &fee(ORDER, T1)).await;
    let earlier = fee(ORDER, T0);

    let mut transaction = pool.begin().await.expect("begin");
    events::insert_if_new(
        &mut *transaction,
        &EventRecord {
            id: earlier.event_id.clone(),
            pubkey: earlier.pubkey.clone(),
            kind: 8383,
            created_at: earlier.created_at,
            d_tag: None,
            raw_json: "{}".to_string(),
            relay_url: RELAY.to_string(),
            seen_at: earlier.created_at,
        },
    )
    .await
    .expect("event");
    insert(&mut *transaction, &earlier).await.expect("fee");
    transaction.commit().await.expect("commit");

    // The fee that arrived second is the earlier one, so it takes over as
    // canonical — the refresh ran on the transaction's own connection.
    assert_eq!(
        flags(&pool, ORDER).await,
        vec![(format!("{T0:064x}"), false), (format!("{T1:064x}"), true),]
    );
}

#[tokio::test]
async fn an_order_with_no_fee_has_none() {
    let pool = migrated().await;

    assert!(for_order(&pool, ORDER).await.expect("read").is_empty());
}

#[tokio::test]
async fn a_fee_with_no_destination_or_network_round_trips() {
    // Both describe where the payment went, not that it happened, and the
    // parser leaves them optional for exactly that reason.
    let pool = migrated().await;
    let mut bare = fee(ORDER, T0);
    bare.destination = None;
    bare.network = None;

    ingest(&pool, &bare).await;

    let stored = for_order(&pool, ORDER).await.expect("read");
    assert_eq!(stored[0].fee, bare);
}
