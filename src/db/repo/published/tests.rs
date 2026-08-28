//! Publication history over a migrated database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;

async fn migrated() -> SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

const NOW: i64 = 1_787_800_000;
const A_WEEK: i64 = 604_800;

fn restated(hash: &str, revision: u32, updated_at: i64, because: &str) -> Previous {
    Previous::restated(
        hash,
        revision,
        updated_at,
        Restatement {
            at: updated_at,
            because: because.to_string(),
        },
    )
    .expect("a revision above the first")
}

// ---- published_documents

#[tokio::test]
async fn an_archive_that_never_published_has_no_history() {
    let pool = migrated().await;

    assert!(all(&pool).await.expect("read").is_empty());
}

#[tokio::test]
async fn a_first_publication_round_trips() {
    let pool = migrated().await;
    let previous = Previous::First {
        hash: "3f9a".to_string(),
        updated_at: NOW,
    };

    record(&pool, "orders:30d", &previous).await.expect("write");

    assert_eq!(
        all(&pool).await.expect("read").get("orders:30d"),
        Some(&previous)
    );
}

#[tokio::test]
async fn a_restatement_round_trips_with_its_reason() {
    let pool = migrated().await;
    let previous = restated("b710", 4, NOW - A_WEEK, "backfill");

    record(&pool, "series:orders:daily:2026-08", &previous)
        .await
        .expect("write");

    let history = all(&pool).await.expect("read");
    let read = history
        .get("series:orders:daily:2026-08")
        .expect("recorded");
    assert_eq!(read, &previous);
    assert_eq!(read.revision(), 4);
    assert_eq!(
        read.restatement().map(|r| r.because.clone()),
        Some("backfill".to_string())
    );
}

#[tokio::test]
async fn recording_the_same_address_twice_replaces_it() {
    // Every run writes back every document it published, unchanged ones
    // included; the second write of an unchanged document is the same
    // values again, and must not become a second row.
    let pool = migrated().await;
    record(
        &pool,
        "orders:30d",
        &Previous::First {
            hash: "3f9a".to_string(),
            updated_at: NOW - A_WEEK,
        },
    )
    .await
    .expect("write");

    let moved = restated("c001", 2, NOW, "rebuild");
    record(&pool, "orders:30d", &moved).await.expect("rewrite");

    let history = all(&pool).await.expect("read");
    assert_eq!(history.len(), 1);
    assert_eq!(history.get("orders:30d"), Some(&moved));
}

#[tokio::test]
async fn an_address_this_run_did_not_compute_keeps_its_history() {
    // A partition that fell outside coverage is still on the relay under
    // its own `d`. Forgetting it here would restart its revision at 1 the
    // next time it came back.
    let pool = migrated().await;
    let old = restated("b710", 3, NOW - A_WEEK, "backfill");
    record(&pool, "series:orders:daily:2026-01", &old)
        .await
        .expect("write");

    record(
        &pool,
        "orders:30d",
        &Previous::First {
            hash: "3f9a".to_string(),
            updated_at: NOW,
        },
    )
    .await
    .expect("write another");

    let history = all(&pool).await.expect("read");
    assert_eq!(history.get("series:orders:daily:2026-01"), Some(&old));
}

#[tokio::test]
async fn a_row_claiming_a_revision_it_cannot_justify_is_read_as_a_first_publication() {
    // The database can hold "revision 3, restated for no reason"; the
    // type cannot, and reading it as revision 3 would publish a
    // restatement with no provenance — the one thing §8 forbids.
    let pool = migrated().await;
    sqlx::query(
        "INSERT INTO published_documents (d, hash, revision, updated_at) VALUES (?, ?, ?, ?)",
    )
    .bind("orders:30d")
    .bind("3f9a")
    .bind(3)
    .bind(NOW)
    .execute(&pool)
    .await
    .expect("write a row by hand");

    let history = all(&pool).await.expect("read");

    assert_eq!(
        history.get("orders:30d"),
        Some(&Previous::First {
            hash: "3f9a".to_string(),
            updated_at: NOW,
        })
    );
}

// ---- publication_runs

#[tokio::test]
async fn an_archive_that_never_published_has_no_last_run() {
    let pool = migrated().await;

    assert_eq!(latest_run(&pool).await.expect("read"), None);
}

#[tokio::test]
async fn the_latest_run_is_the_one_with_the_latest_clock() {
    let pool = migrated().await;
    let earlier = Run {
        snapshot_id: "20260820T000000Z".to_string(),
        generated_at: NOW - A_WEEK,
        schema_version: 1,
        first_event_at: Some(NOW - 10 * A_WEEK),
        last_event_at: Some(NOW - A_WEEK),
        events: 4_100,
    };
    let later = Run {
        snapshot_id: "20260827T030640Z".to_string(),
        generated_at: NOW,
        schema_version: 2,
        first_event_at: None,
        last_event_at: None,
        events: 0,
    };

    // Written out of order on purpose: the ordering is the query's job.
    record_run(&pool, &later).await.expect("write");
    record_run(&pool, &earlier).await.expect("write");

    assert_eq!(latest_run(&pool).await.expect("read"), Some(later));
}

#[tokio::test]
async fn a_run_that_read_an_empty_archive_records_that_it_covered_nothing() {
    // `None` at both ends is a fact about the archive, not a missing
    // field — and it is what a later run compares against to see that a
    // backfill has since reached back.
    let pool = migrated().await;
    let run = Run {
        snapshot_id: "20260827T030640Z".to_string(),
        generated_at: NOW,
        schema_version: 1,
        first_event_at: None,
        last_event_at: None,
        events: 0,
    };

    record_run(&pool, &run).await.expect("write");

    let read = latest_run(&pool).await.expect("read").expect("a run");
    assert_eq!(read.first_event_at, None);
    assert_eq!(read.last_event_at, None);
    assert_eq!(read.events, 0);
}

#[tokio::test]
async fn recording_a_run_twice_under_one_snapshot_id_is_one_run() {
    let pool = migrated().await;
    let run = Run {
        snapshot_id: "20260827T030640Z".to_string(),
        generated_at: NOW,
        schema_version: 1,
        first_event_at: Some(NOW - A_WEEK),
        last_event_at: Some(NOW),
        events: 4_200,
    };

    record_run(&pool, &run).await.expect("write");
    record_run(&pool, &run).await.expect("write again");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM publication_runs")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}
