//! The record of what was asked for, over a migrated database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;

async fn migrated() -> SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

const JANUARY: i64 = 1_735_689_600;
const AUGUST: i64 = 1_754_000_000;
const NOW: i64 = 1_756_000_000;

#[tokio::test]
async fn a_kind_nobody_asked_for_has_no_floor() {
    // Arrange
    let pool = migrated().await;

    // Act / Assert
    assert_eq!(indexed_from(&pool, 38383).await.expect("read"), None);
}

#[tokio::test]
async fn a_kind_that_was_asked_for_carries_the_floor_it_was_asked_from() {
    let pool = migrated().await;

    record(&pool, 38383, AUGUST, NOW).await.expect("record");

    assert_eq!(
        indexed_from(&pool, 38383).await.expect("read"),
        Some(AUGUST)
    );
}

#[tokio::test]
async fn a_later_shallower_walk_does_not_unlearn_a_deeper_one() {
    // Arrange: a full walk, then one bounded to a recent window.
    let pool = migrated().await;
    record(&pool, 38383, JANUARY, NOW).await.expect("deep");

    // Act
    record(&pool, 38383, AUGUST, NOW + 1)
        .await
        .expect("shallow");

    // Assert
    assert_eq!(
        indexed_from(&pool, 38383).await.expect("read"),
        Some(JANUARY),
        "the archive still holds what the deeper walk stored"
    );
}

#[tokio::test]
async fn a_floor_before_the_epoch_is_stored_as_the_epoch() {
    let pool = migrated().await;

    record(&pool, 8383, -10, NOW).await.expect("record");

    assert_eq!(indexed_from(&pool, 8383).await.expect("read"), Some(0));
}
