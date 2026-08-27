//! The eight steps of `docs/SPEC.md` §8.1, against a real migrated database.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};
use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::ingest::parse::fixtures::load;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const OTHER_RELAY: &str = "wss://relay.damus.io";
const NOW: i64 = 1_787_800_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

/// A policy that indexes anything Mostro publishes on mainnet, whoever
/// publishes it. Tests that are not about the allow-list start from here.
fn open_policy() -> Policy {
    Policy::new(Vec::<String>::new(), true, [Network::Mainnet])
}

fn pipeline(pool: &SqlitePool, policy: Policy) -> Pipeline {
    Pipeline::new(pool.clone(), policy)
}

/// Takes the whole statement rather than a table name: sqlx only accepts a
/// `'static` string, which is the same guard that keeps a table name from
/// being built out of anything a test computes.
async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(pool)
        .await
        .expect("count")
}

const EVENTS: &str = "SELECT COUNT(*) FROM events";
const ORDER_VERSIONS: &str = "SELECT COUNT(*) FROM order_versions";
const INSTANCES: &str = "SELECT COUNT(*) FROM instances";
const INSTANCE_INFO: &str = "SELECT COUNT(*) FROM instance_info";

/// Re-signs nothing: it only rewrites the content, so the id and the
/// signature no longer describe the event. That is what a tampered event
/// looks like on the wire.
fn tampered(event: &Event) -> Event {
    let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).expect("event json");
    json["content"] = serde_json::Value::String("tampered".into());
    Event::from_json(json.to_string()).expect("re-parse")
}

#[tokio::test]
async fn a_valid_order_lands_in_every_table_it_touches() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");
    let order = parse::order::parse(&event).expect("fixture parses");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
    assert_eq!(count(&pool, EVENTS).await, 1);
    assert_eq!(count(&pool, ORDER_VERSIONS).await, 1);
    assert_eq!(count(&pool, INSTANCES).await, 1);
    let stored = repo::orders::find(&pool, &order.order_id)
        .await
        .expect("read")
        .expect("projection refreshed");
    assert_eq!(stored.final_status, order.status);
}

#[tokio::test]
async fn the_same_event_twice_is_stored_once_and_reported_as_a_duplicate() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");
    let pipeline = pipeline(&pool, open_policy());
    pipeline.ingest(&event, RELAY, NOW).await.expect("first");

    // Act
    let outcome = pipeline.ingest(&event, RELAY, NOW).await.expect("second");

    // Assert
    assert_eq!(outcome, IngestOutcome::Duplicate);
    assert_eq!(count(&pool, EVENTS).await, 1);
    assert_eq!(count(&pool, ORDER_VERSIONS).await, 1);
}

#[tokio::test]
async fn a_tampered_event_is_rejected_and_stored_nowhere() {
    // Arrange
    let pool = migrated().await;
    let event = tampered(&load(38383, "pending_range"));

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(
        outcome,
        IngestOutcome::Rejected(Rejection::InvalidSignature)
    );
    assert_eq!(count(&pool, EVENTS).await, 0);
    assert_eq!(count(&pool, ORDER_VERSIONS).await, 0);
    assert_eq!(count(&pool, INSTANCES).await, 0);
}

#[tokio::test]
async fn an_unlisted_pubkey_is_rejected_when_unknown_instances_are_not_accepted() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");
    let policy = Policy::new(Vec::<String>::new(), false, [Network::Mainnet]);

    // Act
    let outcome = pipeline(&pool, policy)
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(
        outcome,
        IngestOutcome::Rejected(Rejection::UnknownInstance {
            pubkey: event.pubkey.to_hex(),
        })
    );
    assert_eq!(count(&pool, EVENTS).await, 0);
}

#[tokio::test]
async fn a_listed_pubkey_is_accepted_when_unknown_instances_are_not() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");
    let policy = Policy::new([event.pubkey.to_hex()], false, [Network::Mainnet]);

    // Act
    let outcome = pipeline(&pool, policy)
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
}

#[tokio::test]
async fn an_order_from_another_platform_is_rejected() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "other_platform_hodlhodl");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(
        outcome,
        IngestOutcome::Rejected(Rejection::OtherPlatform {
            platform: Some("hodlhodl".into()),
        })
    );
    assert_eq!(count(&pool, EVENTS).await, 0);
}

#[tokio::test]
async fn an_order_on_a_network_that_is_not_indexed_is_skipped() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "success");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(
        outcome,
        IngestOutcome::Rejected(Rejection::OtherNetwork {
            network: Network::Regtest,
        })
    );
    assert_eq!(count(&pool, EVENTS).await, 0);
}

#[tokio::test]
async fn that_same_order_is_indexed_when_its_network_is_configured() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "success");
    let policy = Policy::new(Vec::<String>::new(), true, [Network::Regtest]);

    // Act
    let outcome = pipeline(&pool, policy)
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
}

#[tokio::test]
async fn a_dev_fee_lands_in_its_own_table() {
    // Arrange
    let pool = migrated().await;
    let event = load(8383, "typical");
    let fee = parse::dev_fee::parse(&event).expect("fixture parses");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
    let stored = repo::dev_fees::for_order(&pool, &fee.order_id)
        .await
        .expect("read");
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].is_duplicate);
}

#[tokio::test]
async fn a_dispute_lands_with_its_projection_refreshed() {
    // Arrange
    let pool = migrated().await;
    let event = load(38386, "status_settled");
    let version = parse::dispute::parse(&event).expect("fixture parses");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
    let stored = repo::disputes::find(&pool, &version.dispute_id)
        .await
        .expect("read")
        .expect("projection refreshed");
    assert_eq!(stored.final_status, version.status);
}

#[tokio::test]
async fn instance_info_lands_and_names_the_instance() {
    // Arrange
    let pool = migrated().await;
    let event = load(38385, "typical");
    let name = parse::instance_name(&event).expect("fixture publishes a name");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
    let instance = repo::instances::find(&pool, &event.pubkey.to_hex())
        .await
        .expect("read")
        .expect("instance recorded");
    assert_eq!(instance.name.as_deref(), Some(name.as_str()));
    assert_eq!(count(&pool, INSTANCE_INFO).await, 1);
}

#[tokio::test]
async fn a_kind_with_no_parser_is_rejected_but_stays_in_the_archive() {
    // Arrange
    let pool = migrated().await;
    let event = load(10002, "typical");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(
        outcome,
        IngestOutcome::Rejected(Rejection::UnsupportedKind { kind: 10002 })
    );
    // Archived on purpose: 30078 and 10002 are part of the corpus, they just
    // have no parser yet. Keeping the raw event is what lets a later
    // `rebuild --from-raw` read them without going back to the relays.
    assert_eq!(count(&pool, EVENTS).await, 1);
    assert_eq!(count(&pool, ORDER_VERSIONS).await, 0);
}

#[tokio::test]
async fn an_accepted_event_advances_the_cursor_for_its_relay_and_kind() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");

    // Act
    pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    let cursor = repo::sync_state::get(&pool, RELAY, 38383)
        .await
        .expect("read")
        .expect("cursor advanced");
    assert_eq!(cursor.last_created_at, event.created_at.as_secs() as i64);
}

#[tokio::test]
async fn a_rejected_event_leaves_the_cursor_where_it_was() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "other_platform_hodlhodl");

    // Act
    pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    let cursor = repo::sync_state::get(&pool, RELAY, 38383)
        .await
        .expect("read");
    assert_eq!(cursor, None);
}

#[tokio::test]
async fn a_malformed_event_is_rejected_but_stays_in_the_archive() {
    // Arrange
    let pool = migrated().await;
    // Signed for real, so it clears every filter, but missing every tag a
    // 38383 needs. This is what a daemon publishing a shape bestiario does
    // not understand would look like.
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(parse::order::KIND), "")
        .tags([
            Tag::parse(["y", MOSTRO]).expect("y tag"),
            Tag::parse(["network", Network::Mainnet.as_str()]).expect("network tag"),
        ])
        .finalize(&keys)
        .expect("sign");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert!(matches!(
        outcome,
        IngestOutcome::Rejected(Rejection::Malformed(_))
    ));
    assert_eq!(count(&pool, EVENTS).await, 1);
    assert_eq!(count(&pool, ORDER_VERSIONS).await, 0);
}

#[tokio::test]
async fn a_duplicate_from_a_second_relay_advances_that_relay_s_cursor() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "pending_range");
    let pipeline = pipeline(&pool, open_policy());
    pipeline.ingest(&event, RELAY, NOW).await.expect("first");

    // Act
    let outcome = pipeline
        .ingest(&event, OTHER_RELAY, NOW)
        .await
        .expect("second relay");

    // Assert
    assert_eq!(outcome, IngestOutcome::Duplicate);
    assert_eq!(count(&pool, EVENTS).await, 1);
    // Cursors are per relay: the second relay has genuinely delivered this
    // far, and leaving it at zero would make it re-send the same backlog.
    let cursor = repo::sync_state::get(&pool, OTHER_RELAY, 38383)
        .await
        .expect("read")
        .expect("cursor advanced");
    assert_eq!(cursor.last_created_at, event.created_at.as_secs() as i64);
}

#[tokio::test]
async fn a_rate_snapshot_is_stored_in_the_rates_table() {
    // Arrange
    let pool = migrated().await;
    let event = load(30078, "typical");

    // Act
    let outcome = pipeline(&pool, open_policy())
        .ingest(&event, RELAY, NOW)
        .await
        .expect("ingest");

    // Assert
    assert_eq!(outcome, IngestOutcome::Stored);
    let snapshots = crate::db::repo::rates::all(&pool).await.expect("rates");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].event_id, event.id.to_hex());
    assert!(snapshots[0].rates.contains_key("USD"));
}
