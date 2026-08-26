//! Order versions and the projection derived from them, against a real
//! migrated SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::fixtures::load;
use crate::ingest::parse::order::{Direction, FiatAmount, Status};
use crate::network::Network;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const PUBKEY: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const ORDER: &str = "0f4f0a1e-9c22-4b7e-9c19-2a6f4b1d2e30";
/// Four publications of one order, ten minutes apart in the maker's clock.
const T0: i64 = 1_787_700_000;
const T1: i64 = 1_787_700_600;
const T2: i64 = 1_787_701_200;
const T3: i64 = 1_787_701_800;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

/// A version of [`ORDER`] in `status`, published at `created_at`.
///
/// The event id is derived from the timestamp so that every version is a
/// distinct row, as it is on the relays.
fn version(created_at: i64, status: Status) -> OrderVersion {
    OrderVersion {
        event_id: format!("{created_at:064x}"),
        order_id: ORDER.to_string(),
        pubkey: PUBKEY.to_string(),
        created_at,
        direction: Direction::Sell,
        status,
        fiat_code: "VES".to_string(),
        amount_sats: 21_000,
        fiat: FiatAmount::Fixed(100.0),
        payment_methods: vec!["face to face".to_string()],
        premium: 5.0,
        network: Some(Network::Mainnet),
        expires_at: created_at + 900,
    }
}

/// Stores the `events` row a version's foreign key needs, then the version.
async fn store(pool: &SqlitePool, version: &OrderVersion) {
    let record = EventRecord {
        id: version.event_id.clone(),
        pubkey: version.pubkey.clone(),
        kind: 38383,
        created_at: version.created_at,
        d_tag: Some(version.order_id.clone()),
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: version.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    insert_version(pool, version).await.expect("version");
}

/// What the pipeline does for one order event: the version, then the refresh.
async fn ingest(pool: &SqlitePool, version: &OrderVersion) {
    store(pool, version).await;
    refresh_projection(pool, &version.order_id)
        .await
        .expect("refresh");
}

async fn projection(pool: &SqlitePool) -> Order {
    find(pool, ORDER).await.expect("find").expect("projected")
}

#[tokio::test]
async fn a_captured_order_round_trips_through_the_version_table() {
    // Arrange
    let pool = migrated().await;
    let parsed = crate::ingest::parse::order::parse(&load(38383, "success")).expect("parse");

    // Act
    store(&pool, &parsed).await;

    // Assert
    let stored = versions(&pool, &parsed.order_id).await.expect("versions");
    assert_eq!(stored, vec![parsed]);
}

#[tokio::test]
async fn a_range_order_keeps_its_bounds_and_no_single_amount() {
    let pool = migrated().await;
    let parsed = crate::ingest::parse::order::parse(&load(38383, "pending_range")).expect("parse");

    store(&pool, &parsed).await;

    let stored = versions(&pool, &parsed.order_id).await.expect("versions");
    assert_eq!(stored, vec![parsed]);
}

#[tokio::test]
async fn the_same_version_twice_leaves_one_row() {
    // The dedup gate of SPEC 8.1 step 6 is the events table, but a rebuild
    // replays versions directly and must stay idempotent.
    let pool = migrated().await;
    let version = version(T0, Status::Pending);

    store(&pool, &version).await;
    insert_version(&pool, &version).await.expect("replay");

    assert_eq!(versions(&pool, ORDER).await.expect("versions").len(), 1);
}

#[tokio::test]
async fn the_projection_of_a_single_pending_version_is_that_version() {
    let pool = migrated().await;

    ingest(&pool, &version(T0, Status::Pending)).await;

    let order = projection(&pool).await;
    assert_eq!(order.order_id, ORDER);
    assert_eq!(order.pubkey, PUBKEY);
    assert_eq!(order.final_status, Status::Pending);
    assert_eq!(order.first_seen_at, T0);
    assert_eq!(order.last_updated_at, T0);
    assert_eq!(order.success_at, None);
    assert_eq!(order.canceled_at, None);
}

#[tokio::test]
async fn the_latest_version_wins_and_the_first_success_dates_the_sale() {
    let pool = migrated().await;

    for (at, status) in [
        (T0, Status::Pending),
        (T1, Status::InProgress),
        (T2, Status::Success),
    ] {
        ingest(&pool, &version(at, status)).await;
    }

    let order = projection(&pool).await;
    assert_eq!(order.final_status, Status::Success);
    assert_eq!(order.first_seen_at, T0);
    assert_eq!(order.last_updated_at, T2);
    assert_eq!(order.success_at, Some(T2));
    assert_eq!(order.canceled_at, None);
}

#[tokio::test]
async fn out_of_order_arrival_yields_the_same_projection() {
    // Backfill walks backwards, so the success version routinely lands before
    // the pending one. The projection is recomputed from the whole history
    // rather than patched, which is what makes the two orders agree.
    let pool = migrated().await;

    for (at, status) in [
        (T2, Status::Success),
        (T0, Status::Pending),
        (T1, Status::InProgress),
    ] {
        ingest(&pool, &version(at, status)).await;
    }

    let order = projection(&pool).await;
    assert_eq!(order.final_status, Status::Success);
    assert_eq!(order.first_seen_at, T0);
    assert_eq!(order.last_updated_at, T2);
    assert_eq!(order.success_at, Some(T2));
}

#[tokio::test]
async fn a_pending_order_that_is_canceled_gets_no_success_date() {
    // SPEC 7: `pending -> canceled` is the expiry path, and it never moved
    // money. Dating it as a sale would inflate completed volume.
    let pool = migrated().await;

    ingest(&pool, &version(T0, Status::Pending)).await;
    ingest(&pool, &version(T1, Status::Canceled)).await;

    let order = projection(&pool).await;
    assert_eq!(order.final_status, Status::Canceled);
    assert_eq!(order.canceled_at, Some(T1));
    assert_eq!(order.success_at, None);
}

#[tokio::test]
async fn a_status_reached_twice_is_dated_by_the_first_version_that_reached_it() {
    // An instance republishes the same status when another field changes; the
    // sale happened at the first one.
    let pool = migrated().await;

    for (at, status) in [
        (T0, Status::Pending),
        (T1, Status::Success),
        (T2, Status::Success),
    ] {
        ingest(&pool, &version(at, status)).await;
    }

    let order = projection(&pool).await;
    assert_eq!(order.success_at, Some(T1));
    assert_eq!(order.last_updated_at, T2);
}

#[tokio::test]
async fn the_mutable_fields_come_from_the_latest_version() {
    let pool = migrated().await;
    let mut later = version(T1, Status::InProgress);
    later.amount_sats = 42_000;
    later.premium = -2.5;
    later.payment_methods = vec!["revolut".to_string(), "sepa".to_string()];

    ingest(&pool, &version(T0, Status::Pending)).await;
    ingest(&pool, &later).await;

    let order = projection(&pool).await;
    assert_eq!(order.amount_sats, 42_000);
    assert_eq!(order.premium, -2.5);
    assert_eq!(order.payment_methods, vec!["revolut", "sepa"]);
}

#[tokio::test]
async fn a_range_order_projects_no_single_fiat_amount() {
    let pool = migrated().await;
    let mut ranged = version(T0, Status::Pending);
    ranged.fiat = FiatAmount::Range {
        min: 10.0,
        max: 50.0,
    };

    ingest(&pool, &ranged).await;

    assert_eq!(projection(&pool).await.fiat_amount, None);
}

#[tokio::test]
async fn refreshing_twice_leaves_the_same_row() {
    let pool = migrated().await;

    ingest(&pool, &version(T0, Status::Pending)).await;
    refresh_projection(&pool, ORDER).await.expect("refresh");
    let first = projection(&pool).await;
    refresh_projection(&pool, ORDER).await.expect("refresh");

    assert_eq!(projection(&pool).await, first);
    assert_eq!(count(&pool).await, 1);
}

#[tokio::test]
async fn refreshing_an_order_with_no_versions_is_a_no_op() {
    // `rebuild --from-raw` wipes the version tables first; a refresh that
    // wrote an empty row would resurrect an order that no longer exists.
    let pool = migrated().await;

    refresh_projection(&pool, ORDER).await.expect("refresh");

    assert_eq!(find(&pool, ORDER).await.expect("find"), None);
}

#[tokio::test]
async fn versions_of_one_order_do_not_include_another() {
    let pool = migrated().await;
    let mut other = version(T3, Status::Pending);
    other.order_id = "9c1b1f7a-3d55-4a6e-8f21-0b7c5e2d4a91".to_string();

    ingest(&pool, &version(T0, Status::Pending)).await;
    ingest(&pool, &other).await;

    assert_eq!(versions(&pool, ORDER).await.expect("versions").len(), 1);
    assert_eq!(count(&pool).await, 2);
}

async fn count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM orders")
        .fetch_one(pool)
        .await
        .expect("count")
}
