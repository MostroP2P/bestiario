//! The backwards walk, against the local relay of `nostr-sdk`.
//!
//! Seeded with the captured fixtures rather than with generated events, so
//! that what is walked has the timestamps, kinds and tags the network really
//! publishes — including the ones the pipeline turns away.

use nostr_sdk::prelude::MockRelay;
use sqlx::SqlitePool;

use super::*;
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
