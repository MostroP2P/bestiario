//! The resume cursor, against a real migrated SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const OTHER_RELAY: &str = "wss://relay.damus.io";
const ORDERS: u16 = 38383;
const DEV_FEES: u16 = 8383;
/// Two event clocks, and two wall clocks that do not track them.
const EARLIER: i64 = 1_787_700_000;
const LATER: i64 = 1_787_703_600;
const RUN_ONE: i64 = 1_790_000_000;
const RUN_TWO: i64 = 1_790_000_600;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

async fn cursor(pool: &SqlitePool, relay: &str, kind: u16) -> Option<Cursor> {
    get(pool, relay, kind).await.expect("get")
}

#[tokio::test]
async fn an_unread_relay_has_no_cursor() {
    // Arrange
    let pool = migrated().await;

    // Act & Assert — nothing to resume from, so backfill starts at its floor.
    assert_eq!(cursor(&pool, RELAY, ORDERS).await, None);
}

#[tokio::test]
async fn the_first_advance_creates_the_cursor() {
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, EARLIER, RUN_ONE)
        .await
        .expect("advance");

    let stored = cursor(&pool, RELAY, ORDERS).await.expect("stored");
    assert_eq!(stored.last_created_at, EARLIER);
    assert_eq!(stored.updated_at, RUN_ONE);
    assert_eq!(stored.kind, i64::from(ORDERS));
}

#[tokio::test]
async fn a_later_event_moves_the_cursor_forward() {
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, EARLIER, RUN_ONE)
        .await
        .expect("advance");
    advance(&pool, RELAY, ORDERS, LATER, RUN_TWO)
        .await
        .expect("advance");

    let stored = cursor(&pool, RELAY, ORDERS).await.expect("stored");
    assert_eq!(stored.last_created_at, LATER);
    assert_eq!(stored.updated_at, RUN_TWO);
}

#[tokio::test]
async fn an_earlier_event_never_moves_the_cursor_backwards() {
    // Backfill walks into the past. Letting it rewind the cursor would make
    // the next sync re-read everything live sync had already covered.
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");
    advance(&pool, RELAY, ORDERS, EARLIER, RUN_TWO)
        .await
        .expect("advance");

    assert_eq!(
        cursor(&pool, RELAY, ORDERS)
            .await
            .expect("stored")
            .last_created_at,
        LATER
    );
}

#[tokio::test]
async fn a_backwards_advance_still_records_that_the_relay_was_reached() {
    // The event clock does not move, but our own does: a relay that keeps
    // sending old events is still alive, and the operator should see that.
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");
    advance(&pool, RELAY, ORDERS, EARLIER, RUN_TWO)
        .await
        .expect("advance");

    assert_eq!(
        cursor(&pool, RELAY, ORDERS)
            .await
            .expect("stored")
            .updated_at,
        RUN_TWO
    );
}

#[tokio::test]
async fn advancing_to_the_same_timestamp_is_idempotent() {
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");
    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");

    assert_eq!(
        cursor(&pool, RELAY, ORDERS).await.expect("stored"),
        Cursor {
            relay_url: RELAY.to_string(),
            kind: i64::from(ORDERS),
            last_created_at: LATER,
            updated_at: RUN_ONE,
        }
    );
    assert_eq!(all(&pool).await.expect("all").len(), 1);
}

#[tokio::test]
async fn each_kind_on_a_relay_keeps_its_own_cursor() {
    // A relay current on orders may never have carried a dev fee. One cursor
    // per relay would let the second be skipped entirely.
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");

    assert_eq!(cursor(&pool, RELAY, DEV_FEES).await, None);
    assert_eq!(
        cursor(&pool, RELAY, ORDERS)
            .await
            .expect("stored")
            .last_created_at,
        LATER
    );
}

#[tokio::test]
async fn each_relay_keeps_its_own_cursor() {
    let pool = migrated().await;

    advance(&pool, RELAY, ORDERS, LATER, RUN_ONE)
        .await
        .expect("advance");
    advance(&pool, OTHER_RELAY, ORDERS, EARLIER, RUN_ONE)
        .await
        .expect("advance");

    assert_eq!(
        cursor(&pool, OTHER_RELAY, ORDERS)
            .await
            .expect("stored")
            .last_created_at,
        EARLIER
    );
    assert_eq!(all(&pool).await.expect("all").len(), 2);
}

#[tokio::test]
async fn every_cursor_is_listed_by_relay_and_then_kind() {
    let pool = migrated().await;

    for (relay, kind) in [(RELAY, ORDERS), (RELAY, DEV_FEES), (OTHER_RELAY, ORDERS)] {
        advance(&pool, relay, kind, EARLIER, RUN_ONE)
            .await
            .expect("advance");
    }

    let listed: Vec<(String, i64)> = all(&pool)
        .await
        .expect("all")
        .into_iter()
        .map(|cursor| (cursor.relay_url, cursor.kind))
        .collect();
    assert_eq!(
        listed,
        vec![
            (OTHER_RELAY.to_string(), i64::from(ORDERS)),
            (RELAY.to_string(), i64::from(DEV_FEES)),
            (RELAY.to_string(), i64::from(ORDERS)),
        ]
    );
}
