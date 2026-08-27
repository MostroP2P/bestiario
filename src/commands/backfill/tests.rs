//! The backwards walk, against the local relay of `nostr-sdk`.
//!
//! Seeded with the captured fixtures rather than with generated events, so
//! that what is walked has the timestamps, kinds and tags the network really
//! publishes — including the ones the pipeline turns away.

use clap::Parser as _;
use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, MockRelay, Tag, Timestamp};
use sqlx::SqlitePool;

use super::*;
use crate::cli::Cli;
use crate::db::connect_and_migrate;
use crate::ingest::Policy;
use crate::ingest::parse::fixtures::{for_relay, load};
use crate::network::Network;

const MEMORY: &str = "sqlite::memory:";
const NOW: i64 = 1_787_800_000;

/// Comfortably after every fixture, so an unbounded walk starts above them.
const AFTER_EVERYTHING: i64 = 1_787_800_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn pipeline(pool: &SqlitePool) -> Pipeline {
    Pipeline::new(
        pool.clone(),
        Policy::new(Vec::<String>::new(), true, [Network::Mainnet]),
    )
}

/// A local relay holding `fixtures`, and a client connected to it.
async fn seeded(fixtures: &[(u16, &str)]) -> (MockRelay, RelayClient) {
    let relay = MockRelay::run().await.expect("start the local relay");

    for (kind, name) in fixtures {
        relay
            .add_event(for_relay(&load(*kind, name)))
            .await
            .expect("the local relay accepts a signed event");
    }

    let client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");

    (relay, client)
}

fn range(from: i64) -> Range {
    Range::resolve(Some(from), Some(AFTER_EVERYTHING), NOW).expect("a non-empty window")
}

#[tokio::test]
async fn a_walk_stores_every_event_the_relay_holds_in_the_window() {
    // Arrange
    let pool = migrated().await;
    let (_relay, client) = seeded(&[
        (38383, "pending_range"),
        (38383, "canceled"),
        (38383, "in_progress"),
    ])
    .await;

    // Act
    let counts = Backfill::new(&client, &pipeline(&pool))
        .run(&[38383], range(0), NOW)
        .await
        .expect("walk");

    // Assert
    assert_eq!(counts.stored, 3);
    assert_eq!(counts.rejected, 0);
    // The oldest second is re-read once: the walk keeps asking until a window
    // comes back empty, because a short page is not proof that a relay has
    // nothing older. Dedup absorbs the repeat.
    assert_eq!(counts.duplicate, 1);
}

#[tokio::test]
async fn a_window_smaller_than_the_backlog_is_walked_page_by_page() {
    // One event per request forces the walk to step backwards three times
    // rather than taking the whole backlog in a single window.
    let pool = migrated().await;
    let (_relay, client) = seeded(&[
        (38383, "pending_range"),
        (38383, "canceled"),
        (38383, "in_progress"),
    ])
    .await;

    let counts = Backfill::new(&client, &pipeline(&pool))
        .with_window_limit(1)
        .run(&[38383], range(0), NOW)
        .await
        .expect("walk");

    assert_eq!(counts.stored, 3);
}

#[tokio::test]
async fn the_walk_does_not_reach_below_the_floor_it_was_given() {
    // Arrange: a floor between the two oldest fixtures and the rest.
    let pool = migrated().await;
    let (_relay, client) = seeded(&[
        (38383, "pending_range_with_fixed_sats"),    // 1787723816
        (38383, "pending_multiple_payment_methods"), // 1787737743
        (38383, "pending_range"),                    // 1787740678
    ])
    .await;

    // Act
    let counts = Backfill::new(&client, &pipeline(&pool))
        .with_window_limit(1)
        .run(&[38383], range(1_787_730_000), NOW)
        .await
        .expect("walk");

    // Assert
    assert_eq!(counts.stored, 2);
}

#[tokio::test]
async fn walking_the_same_backlog_twice_stores_it_once() {
    // Arrange
    let pool = migrated().await;
    let (_relay, client) = seeded(&[(38383, "pending_range"), (38383, "canceled")]).await;
    let pipeline = pipeline(&pool);
    let backfill = Backfill::new(&client, &pipeline);
    backfill
        .run(&[38383], range(0), NOW)
        .await
        .expect("first walk");

    // Act
    let counts = backfill
        .run(&[38383], range(0), NOW)
        .await
        .expect("second walk");

    // Assert: two events, plus the oldest second re-read on the way to the
    // empty window that ends the walk.
    assert_eq!(counts.stored, 0);
    assert_eq!(counts.duplicate, 3);
}

#[tokio::test]
async fn an_event_the_pipeline_turns_away_is_counted_as_rejected() {
    // Arrange
    let pool = migrated().await;
    let (_relay, client) =
        seeded(&[(38383, "pending_range"), (38383, "other_platform_hodlhodl")]).await;

    // Act
    let counts = Backfill::new(&client, &pipeline(&pool))
        .run(&[38383], range(0), NOW)
        .await
        .expect("walk");

    // Assert: the rejected event is the oldest of the two, so the overlapping
    // window reads it a second time and turns it away again. Rejections are
    // decided before the archive, so there is no dedup to absorb them.
    assert_eq!(counts.stored, 1);
    assert_eq!(counts.rejected, 2);
}

#[tokio::test]
async fn every_requested_kind_is_walked() {
    // Arrange
    let pool = migrated().await;
    let (_relay, client) = seeded(&[
        (38383, "pending_range"),
        (8383, "typical"),
        (38386, "status_settled"),
        (38385, "typical"),
    ])
    .await;

    // Act
    let counts = Backfill::new(&client, &pipeline(&pool))
        .run(&crate::nostr::filters::INDEXED_KINDS, range(0), NOW)
        .await
        .expect("walk");

    // Assert
    assert_eq!(counts.stored, 4);
}

#[tokio::test]
async fn a_relay_that_cannot_be_reached_does_not_abort_the_run() {
    // Arrange: one live relay and one that is not there.
    let pool = migrated().await;
    let relay = MockRelay::run().await.expect("start the local relay");
    relay
        .add_event(for_relay(&load(38383, "pending_range")))
        .await
        .expect("seed");
    let client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    relay.shutdown();

    // Act: the relay is gone, so every window fails.
    let counts = Backfill::new(&client, &pipeline(&pool))
        .run(&[38383], range(0), NOW)
        .await
        .expect("walk");

    // Assert: a failed relay is a run that stored nothing, not a panic.
    assert_eq!(counts.stored, 0);
}

#[tokio::test]
async fn a_page_with_room_to_spare_does_not_end_the_walk() {
    // A relay may cap its reply below the limit the client asked for, so a
    // short page says nothing about what it still holds. `MockRelay` honours
    // the limit exactly and cannot be made to cap, so what is asserted here is
    // the request the walk makes *after* the short page: the oldest second is
    // read a second time, which only happens if the walk did not stop there.
    let pool = migrated().await;
    let (_relay, client) = seeded(&[(38383, "pending_range"), (38383, "canceled")]).await;

    let counts = Backfill::new(&client, &pipeline(&pool))
        .with_window_limit(500)
        .run(&[38383], range(0), NOW)
        .await
        .expect("walk");

    assert_eq!(counts.stored, 2);
    assert_eq!(counts.duplicate, 1);
}

#[tokio::test]
async fn a_database_that_cannot_be_written_to_fails_the_run() {
    // Arrange: a closed pool is every later write failing at once, which is
    // what a full disk or a locked database looks like from here.
    let pool = migrated().await;
    let (_relay, client) = seeded(&[(38383, "pending_range")]).await;
    let pipeline = pipeline(&pool);
    pool.close().await;

    // Act
    let result = Backfill::new(&client, &pipeline)
        .run(&[38383], range(0), NOW)
        .await;

    // Assert: reported, not counted. A summary that omitted the event and
    // exited zero would claim an archive that is not on disk, and §8.1 reads
    // the archive row first, so the next run would not retry it either.
    let error = result.expect_err("a failed write ends the run");
    assert!(
        error.to_string().contains("storing event"),
        "unexpected error: {error}"
    );
}

/// The settings a discovery test runs under: one relay configured, and the
/// operator asking bestiario to follow what the instances advertise.
fn settings_for(relay: &str, discover: bool) -> Settings {
    Settings::from_toml_str(&format!(
        r#"
[nostr]
relays = ["{relay}"]
discover_relays = {discover}

[indexer]
instances = []
accept_unknown_instances = true
networks = ["mainnet"]

[database]
url = "sqlite::memory:"
"#
    ))
    .expect("valid settings")
}

/// A kind 10002 naming `url` as a relay the signer publishes to.
fn relay_list_naming(keys: &Keys, url: &str) -> Event {
    EventBuilder::new(Kind::from_u16(10002), "")
        .tag(Tag::parse(["r", url]).expect("well-formed tag"))
        .custom_created_at(Timestamp::from_secs(NOW as u64 - 100))
        .finalize(keys)
        .expect("sign")
}

async fn stored_event_ids(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT id FROM events")
        .fetch_all(pool)
        .await
        .expect("ids")
}

#[tokio::test]
async fn a_relay_named_by_a_list_walked_this_run_is_walked_this_run() {
    // Arrange: the configured relay carries only an instance's relay list,
    // and the relay that list names is the only one holding the order. A
    // client built once from the table as it stood at startup would never
    // dial it, and the order would wait for a second invocation.
    let keys = Keys::generate();
    let advertised = MockRelay::run().await.expect("start the advertised relay");
    let advertised_url = advertised.url().await.to_string();
    let order = for_relay(&load(38383, "pending_range"));
    advertised
        .add_event(order.clone())
        .await
        .expect("the order lives only there");

    let configured = MockRelay::run().await.expect("start the configured relay");
    let configured_url = configured.url().await.to_string();
    configured
        .add_event(relay_list_naming(&keys, &advertised_url))
        .await
        .expect("the list lives here");

    let pool = migrated().await;
    let settings = settings_for(&configured_url, true);
    let cli = Cli::try_parse_from(["bestiario", "backfill"]).expect("parses");
    let context = Context {
        settings: &settings,
        pool: &pool,
        cli: &cli,
    };
    let pipeline = Pipeline::new(
        pool.clone(),
        // The list's signer is listed, so §8.1 step 4b vouches for its
        // untagged kind without a tagged event having to come first.
        Policy::new(vec![keys.public_key().to_hex()], true, [Network::Mainnet]),
    );
    let mut client = RelayClient::connect(&[configured_url])
        .await
        .expect("connect");

    // Act
    let counts = walk_with_discovery(
        &context,
        &mut client,
        &pipeline,
        &filters::INDEXED_KINDS,
        range(0),
        NOW,
    )
    .await
    .expect("the walk");

    // Assert: the advertised relay is in play, and what only it held is in
    // the archive.
    assert_eq!(client.relays().len(), 2, "the advertised relay was dialled");
    assert!(
        stored_event_ids(&pool).await.contains(&order.id.to_hex()),
        "the order only the advertised relay held is archived"
    );
    assert!(counts.stored >= 2, "the list and the order: {counts:?}");
}

#[tokio::test]
async fn with_discovery_off_an_advertised_relay_is_not_dialled() {
    // The flag is the operator's decision, and a relay list is a third
    // party's claim. Off, the run follows exactly what was configured.
    let keys = Keys::generate();
    let advertised = MockRelay::run().await.expect("start the advertised relay");
    let order = for_relay(&load(38383, "pending_range"));
    advertised.add_event(order.clone()).await.expect("seed");

    let configured = MockRelay::run().await.expect("start the configured relay");
    let configured_url = configured.url().await.to_string();
    configured
        .add_event(relay_list_naming(
            &keys,
            &advertised.url().await.to_string(),
        ))
        .await
        .expect("seed");

    let pool = migrated().await;
    let settings = settings_for(&configured_url, false);
    let cli = Cli::try_parse_from(["bestiario", "backfill"]).expect("parses");
    let context = Context {
        settings: &settings,
        pool: &pool,
        cli: &cli,
    };
    let pipeline = Pipeline::new(
        pool.clone(),
        Policy::new(vec![keys.public_key().to_hex()], true, [Network::Mainnet]),
    );
    let mut client = RelayClient::connect(&[configured_url])
        .await
        .expect("connect");

    walk_with_discovery(
        &context,
        &mut client,
        &pipeline,
        &filters::INDEXED_KINDS,
        range(0),
        NOW,
    )
    .await
    .expect("the walk");

    assert_eq!(client.relays().len(), 1, "only what was configured");
    assert!(
        !stored_event_ids(&pool).await.contains(&order.id.to_hex()),
        "and nothing from the relay it did not dial"
    );
}
