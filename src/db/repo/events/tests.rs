//! The dedup gate, against a real migrated SQLite database.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::ingest::parse::fixtures::load;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const SEEN_AT: i64 = 1_787_800_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

async fn stored_ids(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT id FROM events ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read events")
}

#[tokio::test]
async fn an_unseen_event_is_stored_and_reported_as_new() {
    // Arrange
    let pool = migrated().await;
    let event = load(38383, "success");
    let record = EventRecord::new(&event, RELAY, SEEN_AT);

    // Act
    let stored = insert_if_new(&pool, &record).await.expect("insert");

    // Assert
    assert!(stored);
    assert_eq!(stored_ids(&pool).await, vec![event.id.to_hex()]);
}

#[tokio::test]
async fn the_same_event_twice_leaves_one_row_and_reports_the_second_as_known() {
    // The whole point of the gate: three relays delivering one event must not
    // count it three times, and re-running a backfill must be cheap.
    let pool = migrated().await;
    let record = EventRecord::new(&load(38383, "success"), RELAY, SEEN_AT);

    assert!(insert_if_new(&pool, &record).await.expect("first"));
    assert!(!insert_if_new(&pool, &record).await.expect("second"));

    assert_eq!(stored_ids(&pool).await.len(), 1);
}

#[tokio::test]
async fn a_second_relay_does_not_overwrite_the_first_sighting() {
    // `relay_url` records where the event was first seen. Letting a later
    // delivery rewrite it would make the column mean "the last relay that
    // happened to send it", which nothing wants to know.
    let pool = migrated().await;
    let event = load(38383, "success");
    let first = EventRecord::new(&event, RELAY, SEEN_AT);
    let second = EventRecord::new(&event, "wss://nos.lol", SEEN_AT + 60);

    insert_if_new(&pool, &first).await.expect("first");
    insert_if_new(&pool, &second).await.expect("second");

    let (relay, seen_at) =
        sqlx::query_as::<_, (String, i64)>("SELECT relay_url, seen_at FROM events")
            .fetch_one(&pool)
            .await
            .expect("read row");
    assert_eq!(relay, RELAY);
    assert_eq!(seen_at, SEEN_AT);
}

#[tokio::test]
async fn the_stored_json_is_the_event_itself() {
    // `rebuild --from-raw` re-derives every table from this column, so it has
    // to round-trip back into the same event, signature included.
    let pool = migrated().await;
    let event = load(38383, "pending_range");

    insert_if_new(&pool, &EventRecord::new(&event, RELAY, SEEN_AT))
        .await
        .expect("insert");

    let raw: String = sqlx::query_scalar("SELECT raw_json FROM events")
        .fetch_one(&pool)
        .await
        .expect("read raw_json");
    let round_tripped = nostr_sdk::prelude::Event::from_json(&raw).expect("parse");
    assert_eq!(round_tripped, event);
    assert!(round_tripped.verify().is_ok());
}

#[tokio::test]
async fn the_d_tag_is_stored_for_addressable_kinds_and_null_for_the_rest() {
    let pool = migrated().await;
    let order = load(38383, "success");
    let dev_fee = load(8383, "typical");

    for event in [&order, &dev_fee] {
        insert_if_new(&pool, &EventRecord::new(event, RELAY, SEEN_AT))
            .await
            .expect("insert");
    }

    let d_tags =
        sqlx::query_as::<_, (String, Option<String>)>("SELECT id, d_tag FROM events ORDER BY kind")
            .fetch_all(&pool)
            .await
            .expect("read d_tags");
    let for_dev_fee = d_tags
        .iter()
        .find(|(id, _)| *id == dev_fee.id.to_hex())
        .expect("dev fee row");
    let for_order = d_tags
        .iter()
        .find(|(id, _)| *id == order.id.to_hex())
        .expect("order row");

    assert_eq!(for_dev_fee.1, None, "8383 publishes no d tag");
    assert!(for_order.1.is_some(), "38383 is addressable");
}

#[tokio::test]
async fn exists_reports_what_insert_if_new_stored() {
    let pool = migrated().await;
    let event = load(38386, "status_settled");
    let record = EventRecord::new(&event, RELAY, SEEN_AT);

    assert!(!exists(&pool, &record.id).await.expect("before"));
    insert_if_new(&pool, &record).await.expect("insert");
    assert!(exists(&pool, &record.id).await.expect("after"));
}

#[tokio::test]
async fn two_versions_of_the_same_order_are_two_rows() {
    // Addressable kinds are replaced on the relay but accumulated here: the
    // lifecycle of an order is only reconstructible from every version of it.
    let pool = migrated().await;
    let first = EventRecord::new(&load(38383, "pending_range"), RELAY, SEEN_AT);
    let second = EventRecord::new(&load(38383, "in_progress"), RELAY, SEEN_AT);

    insert_if_new(&pool, &first).await.expect("first");
    insert_if_new(&pool, &second).await.expect("second");

    assert_eq!(stored_ids(&pool).await.len(), 2);
}

// How far back the archive can speak for (issue #53).

/// 2026-01-01 and 2026-08-26, roughly: a dev fee kept for a year and an
/// order kept for a fortnight, which is the asymmetry that matters.
const JANUARY: i64 = 1_767_225_600;
const AUGUST: i64 = 1_787_702_400;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "6320ee5e2b6f0e0e0a5d3f5f8e8a3c2b1d0f9e8d7c6b5a4938271605f4e3d2c1";

/// Every instance, every network: what a report with no `--instance` asks.
const EVERYTHING: Scope = Scope {
    pubkey: None,
    networks: Vec::new(),
};

fn only(pubkey: &str) -> Scope {
    Scope {
        pubkey: Some(pubkey.to_string()),
        networks: Vec::new(),
    }
}

/// Records that `kind` was asked for from the beginning, which is what a
/// full backfill or a first `sync` does.
async fn asked_for(pool: &SqlitePool, kind: u16) {
    crate::db::repo::indexed_kinds::record(pool, kind, 0, AUGUST)
        .await
        .expect("record");
}

async fn stored(pool: &SqlitePool, id: &str, kind: i64, created_at: i64) {
    stored_by(pool, id, kind, created_at, ALPHA).await;
}

async fn stored_by(pool: &SqlitePool, id: &str, kind: i64, created_at: i64, pubkey: &str) {
    let record = EventRecord {
        id: id.to_string(),
        pubkey: pubkey.to_string(),
        kind,
        created_at,
        d_tag: None,
        raw_json: "{}".to_string(),
        relay_url: RELAY.to_string(),
        seen_at: created_at,
    };
    insert_if_new(pool, &record).await.expect("insert");
}

#[tokio::test]
async fn a_kind_the_relays_expired_earlier_has_a_later_floor_than_the_archive() {
    // Arrange: a first backfill brings January's dev fees and, because the
    // relay keeps orders a fortnight, only August's orders.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    stored(&pool, "order", 38383, AUGUST).await;

    // Act / Assert
    assert_eq!(
        earliest_created_at(&pool, &[38383], &EVERYTHING)
            .await
            .expect("read"),
        Some(AUGUST),
        "January's order-days were never indexed"
    );
    assert_eq!(
        earliest_created_at(&pool, &[8383], &EVERYTHING)
            .await
            .expect("read"),
        Some(JANUARY)
    );
}

#[tokio::test]
async fn a_report_reading_several_kinds_can_only_speak_for_what_it_has_all_of() {
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    stored(&pool, "order", 38383, AUGUST).await;

    let both = earliest_created_at(&pool, &[8383, 38383], &EVERYTHING)
        .await
        .expect("read");

    assert_eq!(both, Some(AUGUST), "the later of the two floors");
}

#[tokio::test]
async fn a_kind_that_was_asked_for_and_never_seen_is_covered_from_when_indexing_began() {
    // Arrange: bestiario asked for disputes from the beginning and saw
    // none. That is a fact about the network, not a hole in the archive.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    asked_for(&pool, 38386).await;

    // Act
    let disputes = earliest_created_at(&pool, &[38386], &EVERYTHING)
        .await
        .expect("read");

    // Assert
    assert_eq!(disputes, Some(JANUARY));
}

#[tokio::test]
async fn a_kind_nobody_ever_asked_for_is_not_a_confirmed_zero() {
    // Arrange: `backfill --kind 8383` populated the archive and left
    // disputes untouched. Their history is unknown, not empty.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    asked_for(&pool, 8383).await;

    // Act
    let disputes = earliest_created_at(&pool, &[38386], &EVERYTHING)
        .await
        .expect("read");

    // Assert
    assert_eq!(
        disputes, None,
        "the report can speak for none of the window"
    );
}

#[tokio::test]
async fn a_report_reading_two_kinds_speaks_for_neither_when_one_was_never_asked_for() {
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    asked_for(&pool, 8383).await;

    let both = earliest_created_at(&pool, &[8383, 38386], &EVERYTHING)
        .await
        .expect("read");

    assert_eq!(both, None);
}

#[tokio::test]
async fn an_instance_scoped_floor_is_that_instance_s_own() {
    // Arrange: Alpha has been indexed since January; Beta was added in
    // August, by which time its older orders had expired off the relays.
    let pool = migrated().await;
    stored_by(&pool, "alpha", 38383, JANUARY, ALPHA).await;
    stored_by(&pool, "beta", 38383, AUGUST, BETA).await;

    // Act / Assert
    assert_eq!(
        earliest_created_at(&pool, &[38383], &only(BETA))
            .await
            .expect("read"),
        Some(AUGUST),
        "Beta's earlier days were never indexed; Alpha's reach is not Beta's"
    );
    assert_eq!(
        earliest_created_at(&pool, &[38383], &only(ALPHA))
            .await
            .expect("read"),
        Some(JANUARY)
    );
}

#[tokio::test]
async fn an_instance_the_archive_holds_nothing_of_can_speak_for_nothing() {
    let pool = migrated().await;
    stored_by(&pool, "alpha", 38383, JANUARY, ALPHA).await;
    asked_for(&pool, 38383).await;

    assert_eq!(
        earliest_created_at(&pool, &[38383], &only(BETA))
            .await
            .expect("read"),
        None
    );
}

#[tokio::test]
async fn an_empty_archive_can_speak_for_nothing() {
    let pool = migrated().await;

    assert_eq!(
        earliest_created_at(&pool, &[38383], &EVERYTHING)
            .await
            .expect("read"),
        None
    );
}

/// Records that `kind` was asked for from `from`, which is what a
/// `--from`-bounded backfill or a resuming `sync` does.
async fn asked_for_from(pool: &SqlitePool, kind: u16, from: i64) {
    crate::db::repo::indexed_kinds::record(pool, kind, from, AUGUST)
        .await
        .expect("record");
}

#[tokio::test]
async fn a_kind_the_relay_keeps_one_of_is_covered_from_when_it_was_asked_for() {
    // Arrange: a relay keeps exactly one kind 30078 per publisher, so a
    // backfill reaching back to January comes home with one stamped today
    // however far back it asked. Its earliest stored event says when this
    // archive started, not how far back the network can be spoken for.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    asked_for_from(&pool, 8383, JANUARY).await;
    stored(&pool, "rates", 30078, AUGUST).await;
    asked_for_from(&pool, 30078, JANUARY).await;

    // Act
    let both = earliest_created_at(&pool, &[8383, 30078], &EVERYTHING)
        .await
        .expect("read");

    // Assert
    assert_eq!(
        both,
        Some(JANUARY),
        "the single copy the relay kept is not a floor"
    );
}

#[tokio::test]
async fn a_single_copy_kind_asked_for_late_still_bounds_the_floor() {
    // Arrange: rates were only ever asked for from August, so January has
    // no rates behind it whatever the archive happens to hold.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    stored(&pool, "rates", 30078, AUGUST).await;
    asked_for_from(&pool, 30078, AUGUST).await;

    // Act / Assert
    assert_eq!(
        earliest_created_at(&pool, &[8383, 30078], &EVERYTHING)
            .await
            .expect("read"),
        Some(AUGUST)
    );
}

#[tokio::test]
async fn a_single_copy_kind_nobody_asked_for_falls_back_to_what_was_stored() {
    // Arrange: nothing recorded the request, so the only evidence of how
    // far back this kind reaches is the event itself. Conservative, and
    // exactly what the floor was before `indexed_kinds` had a row.
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;
    stored(&pool, "rates", 30078, AUGUST).await;

    // Act / Assert
    assert_eq!(
        earliest_created_at(&pool, &[8383, 30078], &EVERYTHING)
            .await
            .expect("read"),
        Some(AUGUST)
    );
}

#[tokio::test]
async fn a_single_copy_kind_neither_asked_for_nor_stored_speaks_for_nothing() {
    let pool = migrated().await;
    stored(&pool, "fee", 8383, JANUARY).await;

    assert_eq!(
        earliest_created_at(&pool, &[8383, 30078], &EVERYTHING)
            .await
            .expect("read"),
        None
    );
}

#[tokio::test]
async fn an_instance_scoped_floor_is_not_lowered_by_a_network_wide_request() {
    // Arrange: the reviewer's worry, made concrete. Alpha was backfilled
    // from the beginning, so `indexed_kinds` records rates as asked for
    // from 0 — a network-wide row, with no instance column to narrow. Beta
    // joined in August. Reading that 0 as Beta's floor would report
    // confirmed zeros for months before Beta existed.
    let pool = migrated().await;
    stored_by(&pool, "alpha-order", 38383, JANUARY, ALPHA).await;
    stored_by(&pool, "alpha-rates", 30078, JANUARY, ALPHA).await;
    asked_for_from(&pool, 30078, 0).await;
    asked_for_from(&pool, 38383, 0).await;
    stored_by(&pool, "beta-order", 38383, AUGUST, BETA).await;
    stored_by(&pool, "beta-rates", 30078, AUGUST, BETA).await;

    // Act
    let beta = earliest_created_at(&pool, &[38383, 30078], &only(BETA))
        .await
        .expect("read");

    // Assert: the scoped floor of the archive itself holds the line — a
    // report can never reach back past the first event of the instance it
    // covers, whatever an unscoped `indexed_kinds` row says.
    assert_eq!(beta, Some(AUGUST));
}
