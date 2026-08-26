//! Dispute versions and their projection, against a real migrated SQLite
//! database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::fixtures::load;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const PUBKEY: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const DISPUTE: &str = "3c9a6d21-84be-4f0a-9d13-7b2c5e8f1a44";
/// When the dispute was opened, and three publications after it.
const OPENED: i64 = 1_787_699_000;
const T0: i64 = 1_787_700_000;
const T1: i64 = 1_787_700_600;
const T2: i64 = 1_787_701_200;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn version(created_at: i64, status: Status) -> DisputeVersion {
    DisputeVersion {
        event_id: format!("{created_at:064x}"),
        dispute_id: DISPUTE.to_string(),
        pubkey: PUBKEY.to_string(),
        created_at,
        status,
        initiator: Some(Initiator::Buyer),
        opened_at: Some(OPENED),
    }
}

async fn store(pool: &SqlitePool, version: &DisputeVersion) {
    let record = EventRecord {
        id: version.event_id.clone(),
        pubkey: version.pubkey.clone(),
        kind: 38386,
        created_at: version.created_at,
        d_tag: Some(version.dispute_id.clone()),
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: version.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    insert_version(pool, version).await.expect("version");
}

async fn ingest(pool: &SqlitePool, version: &DisputeVersion) {
    store(pool, version).await;
    refresh_projection(pool, &version.dispute_id)
        .await
        .expect("refresh");
}

async fn projection(pool: &SqlitePool) -> Dispute {
    find(pool, DISPUTE).await.expect("find").expect("projected")
}

#[tokio::test]
async fn a_captured_dispute_round_trips_through_the_version_table() {
    // Arrange
    let pool = migrated().await;
    let parsed =
        crate::ingest::parse::dispute::parse(&load(38386, "status_settled")).expect("parse");

    // Act
    store(&pool, &parsed).await;

    // Assert
    let stored = versions(&pool, &parsed.dispute_id).await.expect("versions");
    assert_eq!(stored, vec![parsed]);
}

#[tokio::test]
async fn the_same_version_twice_leaves_one_row() {
    let pool = migrated().await;
    let version = version(T0, Status::Initiated);

    store(&pool, &version).await;
    insert_version(&pool, &version).await.expect("replay");

    assert_eq!(versions(&pool, DISPUTE).await.expect("versions").len(), 1);
}

#[tokio::test]
async fn the_projection_of_a_single_version_is_that_version() {
    let pool = migrated().await;

    ingest(&pool, &version(T0, Status::Initiated)).await;

    let dispute = projection(&pool).await;
    assert_eq!(dispute.dispute_id, DISPUTE);
    assert_eq!(dispute.pubkey, PUBKEY);
    assert_eq!(dispute.final_status, Status::Initiated);
    assert_eq!(dispute.initiator, Some(Initiator::Buyer));
    assert_eq!(dispute.opened_at, Some(OPENED));
    assert_eq!(dispute.last_updated_at, T0);
}

#[tokio::test]
async fn the_latest_version_sets_the_status() {
    let pool = migrated().await;

    for (at, status) in [
        (T0, Status::Initiated),
        (T1, Status::InProgress),
        (T2, Status::Settled),
    ] {
        ingest(&pool, &version(at, status)).await;
    }

    let dispute = projection(&pool).await;
    assert_eq!(dispute.final_status, Status::Settled);
    assert_eq!(dispute.last_updated_at, T2);
}

#[tokio::test]
async fn out_of_order_arrival_yields_the_same_projection() {
    // Backfill walks backwards, so the settled version lands first.
    let pool = migrated().await;

    for (at, status) in [
        (T2, Status::Settled),
        (T0, Status::Initiated),
        (T1, Status::InProgress),
    ] {
        ingest(&pool, &version(at, status)).await;
    }

    let dispute = projection(&pool).await;
    assert_eq!(dispute.final_status, Status::Settled);
    assert_eq!(dispute.last_updated_at, T2);
    assert_eq!(dispute.opened_at, Some(OPENED));
}

#[tokio::test]
async fn a_later_version_that_omits_the_initiator_does_not_erase_it() {
    // The tag says nothing rather than saying "nobody": a dispute that had an
    // initiator keeps it, or the initiator split of SPEC 6.7 loses rows the
    // history still holds.
    let pool = migrated().await;
    let mut silent = version(T1, Status::Settled);
    silent.initiator = None;
    silent.opened_at = None;

    ingest(&pool, &version(T0, Status::Initiated)).await;
    ingest(&pool, &silent).await;

    let dispute = projection(&pool).await;
    assert_eq!(dispute.final_status, Status::Settled);
    assert_eq!(dispute.initiator, Some(Initiator::Buyer));
    assert_eq!(dispute.opened_at, Some(OPENED));
}

#[tokio::test]
async fn a_dispute_that_never_names_an_initiator_has_none() {
    let pool = migrated().await;
    let mut anonymous = version(T0, Status::Initiated);
    anonymous.initiator = None;
    anonymous.opened_at = None;

    ingest(&pool, &anonymous).await;

    let dispute = projection(&pool).await;
    assert_eq!(dispute.initiator, None);
    assert_eq!(dispute.opened_at, None);
}

#[tokio::test]
async fn versions_sharing_a_second_are_settled_by_the_lower_event_id() {
    // created_at has one-second resolution. NIP-01 retains the
    // lexicographically lowest event id for an addressable event, so the
    // projected status has to agree with the version the relays keep.
    let pool = migrated().await;
    let mut lower = version(T0, Status::Settled);
    lower.event_id = format!("0{}", "a".repeat(63));
    let mut higher = version(T0, Status::Released);
    higher.event_id = format!("f{}", "a".repeat(63));

    ingest(&pool, &higher).await;
    ingest(&pool, &lower).await;

    assert_eq!(projection(&pool).await.final_status, Status::Settled);
}

#[tokio::test]
async fn refreshing_twice_leaves_the_same_row() {
    let pool = migrated().await;

    ingest(&pool, &version(T0, Status::Initiated)).await;
    let first = projection(&pool).await;
    refresh_projection(&pool, DISPUTE).await.expect("refresh");

    assert_eq!(projection(&pool).await, first);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM disputes")
            .fetch_one(&pool)
            .await
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn refreshing_a_dispute_with_no_versions_is_a_no_op() {
    let pool = migrated().await;

    refresh_projection(&pool, DISPUTE).await.expect("refresh");

    assert_eq!(find(&pool, DISPUTE).await.expect("find"), None);
}
