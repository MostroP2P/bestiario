//! The live follower, against the local relay of `nostr-sdk`.

use std::time::Duration;

use nostr_sdk::local_relay::LocalRelay;
use nostr_sdk::prelude::MockRelay;
use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::ingest::Policy;
use crate::ingest::parse::fixtures::{for_relay, load};
use crate::network::Network;

const MEMORY: &str = "sqlite::memory:";

/// Long enough that a busy CI machine does not fail the test, short enough
/// that a genuine hang is not mistaken for slowness.
const PATIENCE: Duration = Duration::from_secs(10);

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn pipeline(pool: &SqlitePool) -> Pipeline {
    Pipeline::new(
        pool.clone(),
        Policy::new(Vec::<String>::new(), true, [Network::Mainnet]),
    )
}

async fn stored_events(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .expect("count")
}

#[test]
fn a_subscription_resumes_at_its_cursor_rewound_by_the_overlap() {
    // Arrange / Act / Assert
    assert_eq!(resume_from(Some(1_000_000), 3_600), Some(996_400));
}

#[test]
fn a_relay_with_no_cursor_is_asked_for_everything_it_holds() {
    // No cursor is the first run: an open-ended filter, not one starting at
    // the epoch, because the two are the same request and only one of them
    // says so.
    assert_eq!(resume_from(None, 3_600), None);
}

#[test]
fn the_rewind_never_reaches_before_the_epoch() {
    assert_eq!(resume_from(Some(60), 3_600), Some(0));
}

#[test]
fn the_backoff_doubles_from_one_second_and_stops_at_a_minute() {
    assert_eq!(backoff(1), Duration::from_secs(1));
    assert_eq!(backoff(2), Duration::from_secs(2));
    assert_eq!(backoff(4), Duration::from_secs(8));
    assert_eq!(backoff(7), Duration::from_secs(60));
    // Whatever the attempt count reaches, the wait is bounded and finite.
    assert_eq!(backoff(u32::MAX), BACKOFF_MAX);
}

#[tokio::test]
async fn an_event_published_while_sync_runs_is_stored() {
    // Arrange
    let pool = migrated().await;
    let relay = MockRelay::run().await.expect("start the local relay");
    let mut client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    let pipeline = pipeline(&pool);
    let mut sync = Sync::new(&mut client, &pipeline, &pool);
    let event = for_relay(&load(38383, "pending_range"));

    // Act: publish once the subscription is up, then stop as soon as the row
    // is there — no fixed sleep to make the test slow or flaky.
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let publisher = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        relay.add_event(event.clone()).await.expect("publish");

        while stored_events(&pool).await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = stop.send(());
    };
    let follower = sync.follow(async {
        let _ = stopped.await;
    });

    let (counts, ()) = tokio::time::timeout(PATIENCE, async { tokio::join!(follower, publisher) })
        .await
        .expect("the event arrives and sync stops");

    // Assert
    let counts = counts.expect("follow");
    assert_eq!(counts.stored, 1);
    assert_eq!(stored_events(&pool).await, 1);
}

#[tokio::test]
async fn following_advances_the_cursor_so_an_interrupted_run_needs_no_flush() {
    // Arrange
    let pool = migrated().await;
    let relay = MockRelay::run().await.expect("start the local relay");
    let mut client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    let url = relay.url().await.to_string();
    let pipeline = pipeline(&pool);
    let mut sync = Sync::new(&mut client, &pipeline, &pool);
    let event = for_relay(&load(38383, "pending_range"));

    // Act
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let publisher = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        relay.add_event(event.clone()).await.expect("publish");
        while stored_events(&pool).await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = stop.send(());
    };
    let follower = sync.follow(async {
        let _ = stopped.await;
    });
    let (counts, ()) = tokio::time::timeout(PATIENCE, async { tokio::join!(follower, publisher) })
        .await
        .expect("the event arrives and sync stops");
    counts.expect("follow");

    // Assert: the cursor was written by the pipeline as the event landed,
    // before anything asked the run to stop.
    let cursor = repo::sync_state::get(&pool, &url, 38383)
        .await
        .expect("read")
        .expect("the cursor was advanced");
    assert_eq!(cursor.last_created_at, event.created_at.as_secs() as i64);
}

#[tokio::test]
async fn a_shutdown_before_anything_arrives_stops_cleanly() {
    // Arrange
    let pool = migrated().await;
    let relay = MockRelay::run().await.expect("start the local relay");
    let mut client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    let pipeline = pipeline(&pool);

    // Act
    let counts = tokio::time::timeout(
        PATIENCE,
        Sync::new(&mut client, &pipeline, &pool).follow(std::future::ready(())),
    )
    .await
    .expect("sync stops")
    .expect("follow");

    // Assert
    assert_eq!(counts, Counts::default());
}

#[tokio::test]
async fn a_relay_that_was_down_at_startup_is_followed_once_it_answers() {
    // Arrange: one configured relay is down when the indexer starts, another
    // is up. Without a retry the run would ignore the first for as long as
    // the process lived, and never see what only it carries.
    let pool = migrated().await;
    let port = free_port();
    let down = format!("ws://127.0.0.1:{port}");
    let live = MockRelay::run().await.expect("start the local relay");

    let mut client = RelayClient::connect(&[down, live.url().await.to_string()])
        .await
        .expect("one reachable relay is enough");
    assert_eq!(client.relays().len(), 1);

    // The relay comes back before `follow` builds its subscription.
    let recovered = LocalRelay::builder().port(port).build();
    recovered.run().await.expect("start the recovered relay");

    let pipeline = pipeline(&pool);
    let mut sync = Sync::new(&mut client, &pipeline, &pool);
    let event = for_relay(&load(38383, "pending_range"));

    // Act: publish only on the relay that was down.
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let publisher = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        recovered.add_event(event.clone()).await.expect("publish");

        while stored_events(&pool).await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = stop.send(());
    };
    let follower = sync.follow(async {
        let _ = stopped.await;
    });
    let (counts, ()) = tokio::time::timeout(PATIENCE, async { tokio::join!(follower, publisher) })
        .await
        .expect("the recovered relay delivers");

    // Assert
    assert_eq!(counts.expect("follow").stored, 1);
}

#[tokio::test]
async fn a_database_that_stops_answering_ends_the_run() {
    // A relay that fails is retried; a database that fails is not. Spinning
    // on it would leave `sync` looking healthy while storing nothing, which
    // is the failure nobody notices until the numbers are asked for.
    let pool = migrated().await;
    let relay = MockRelay::run().await.expect("start the local relay");
    let mut client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    let pipeline = pipeline(&pool);
    pool.close().await;

    let result = tokio::time::timeout(
        PATIENCE,
        Sync::new(&mut client, &pipeline, &pool).follow(std::future::pending()),
    )
    .await
    .expect("the run ends rather than retrying");

    assert!(result.is_err());
}

/// A port with nothing on it *yet*: bound to find a free one, then released
/// so the relay under test can take it.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}
