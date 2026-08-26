//! The 38385 history and the fee lookup phase 3 needs, against a real migrated
//! SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::fixtures::load;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const PUBKEY: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const OTHER: &str = "00037abd1b8a1a6c1c3f4b9e2d5a7c8e0f1234567890abcdef1234567890abcd";
/// A fee change: 0.6% until JULY, 1% from then on.
const MAY: i64 = 1_777_000_000;
const JULY: i64 = 1_787_000_000;
const AUGUST: i64 = 1_790_000_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn info(pubkey: &str, created_at: i64, fee: Option<f64>) -> InstanceInfo {
    InstanceInfo {
        event_id: format!("{}{created_at:048x}", &pubkey[..16]),
        pubkey: pubkey.to_string(),
        created_at,
        fee,
        max_order_amount: Some(1_000_000),
        min_order_amount: Some(100),
        fiat_currencies: Some("VES,ARS,COP".to_string()),
        mostro_version: Some("0.13.9".to_string()),
        protocol_version: Some("1".to_string()),
        ln_networks: Some("mainnet".to_string()),
        bond_enabled: Some(false),
    }
}

async fn ingest(pool: &SqlitePool, info: &InstanceInfo) {
    let record = EventRecord {
        id: info.event_id.clone(),
        pubkey: info.pubkey.clone(),
        kind: 38385,
        created_at: info.created_at,
        d_tag: Some(info.pubkey.clone()),
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: info.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    insert_version(pool, info).await.expect("version");
}

#[tokio::test]
async fn a_captured_info_round_trips() {
    // Arrange
    let pool = migrated().await;
    let parsed = crate::ingest::parse::info::parse(&load(38385, "typical")).expect("parse");

    // Act
    ingest(&pool, &parsed).await;

    // Assert
    let stored = versions(&pool, &parsed.pubkey).await.expect("versions");
    assert_eq!(stored, vec![parsed]);
}

#[tokio::test]
async fn a_bond_policy_survives_the_round_trip() {
    // SQLite has no boolean, so `bond_enabled` is the one field that has to be
    // converted back out of an INTEGER.
    let pool = migrated().await;
    let parsed =
        crate::ingest::parse::info::parse(&load(38385, "with_bond_policy")).expect("parse");

    ingest(&pool, &parsed).await;

    let stored = versions(&pool, &parsed.pubkey).await.expect("versions");
    assert_eq!(stored, vec![parsed]);
}

#[tokio::test]
async fn the_same_version_twice_leaves_one_row() {
    let pool = migrated().await;
    let version = info(PUBKEY, JULY, Some(0.006));

    ingest(&pool, &version).await;
    insert_version(&pool, &version).await.expect("replay");

    assert_eq!(versions(&pool, PUBKEY).await.expect("versions").len(), 1);
}

#[tokio::test]
async fn the_fee_in_force_is_the_one_published_before_the_trade() {
    // Phase 3 values a trade at the fee that applied when it happened. Using
    // the newest instead would reprice a year of history every time an
    // instance changes its fee.
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, MAY, Some(0.006))).await;
    ingest(&pool, &info(PUBKEY, JULY, Some(0.01))).await;

    // Act & Assert
    let in_june = fee_in_force(&pool, PUBKEY, JULY - 1).await.expect("lookup");
    let in_august = fee_in_force(&pool, PUBKEY, AUGUST).await.expect("lookup");

    assert_eq!(in_june, Some(0.006));
    assert_eq!(in_august, Some(0.01));
}

#[tokio::test]
async fn a_version_exactly_at_the_timestamp_is_in_force() {
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, JULY, Some(0.01))).await;

    assert_eq!(
        fee_in_force(&pool, PUBKEY, JULY).await.expect("lookup"),
        Some(0.01)
    );
}

#[tokio::test]
async fn a_trade_older_than_every_version_has_no_fee_in_force() {
    // Not zero: valuing a trade at a fee nobody had announced yet would invent
    // volume out of an absence.
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, JULY, Some(0.01))).await;

    assert_eq!(
        fee_in_force(&pool, PUBKEY, MAY).await.expect("lookup"),
        None
    );
}

#[tokio::test]
async fn a_version_that_publishes_no_fee_does_not_hide_the_last_one() {
    // Roughly a third of the captured corpus omits some field or other. An
    // info event without a `fee` tag says nothing about the fee.
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, MAY, Some(0.006))).await;
    ingest(&pool, &info(PUBKEY, JULY, None)).await;

    assert_eq!(
        fee_in_force(&pool, PUBKEY, AUGUST).await.expect("lookup"),
        Some(0.006)
    );
}

#[tokio::test]
async fn a_zero_fee_is_a_fee_and_not_an_absence() {
    // One instance in the corpus publishes `fee = 0`. It is a real policy.
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, MAY, Some(0.006))).await;
    ingest(&pool, &info(PUBKEY, JULY, Some(0.0))).await;

    assert_eq!(
        fee_in_force(&pool, PUBKEY, AUGUST).await.expect("lookup"),
        Some(0.0)
    );
}

#[tokio::test]
async fn one_instance_does_not_read_another_instances_fee() {
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, MAY, Some(0.006))).await;

    assert_eq!(
        fee_in_force(&pool, OTHER, AUGUST).await.expect("lookup"),
        None
    );
    assert!(versions(&pool, OTHER).await.expect("versions").is_empty());
}

#[tokio::test]
async fn the_latest_version_is_the_newest_one() {
    let pool = migrated().await;
    ingest(&pool, &info(PUBKEY, JULY, Some(0.01))).await;
    ingest(&pool, &info(PUBKEY, MAY, Some(0.006))).await;

    let latest = latest(&pool, PUBKEY)
        .await
        .expect("latest")
        .expect("stored");
    assert_eq!(latest.created_at, JULY);
    assert_eq!(latest.fee, Some(0.01));
}

#[tokio::test]
async fn an_unknown_instance_has_no_latest_version() {
    let pool = migrated().await;

    assert_eq!(latest(&pool, PUBKEY).await.expect("latest"), None);
}
