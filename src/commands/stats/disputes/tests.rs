//! The command below the print: a seeded database in, a [`Report`] out.

use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{disputes as disputes_repo, instances, orders};
use crate::ingest::parse::dispute::{DisputeVersion, Initiator, Status};
use crate::ingest::parse::order::{self, Direction, FiatAmount, OrderVersion};
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn event(pool: &SqlitePool, id: &str, kind: u16, created_at: i64) {
    let record = EventRecord {
        id: id.to_string(),
        pubkey: ALPHA.to_string(),
        kind: kind.into(),
        created_at,
        d_tag: None,
        raw_json: "{}".to_string(),
        relay_url: "wss://relay.mostro.network".to_string(),
        seen_at: created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
}

async fn dispute(
    pool: &SqlitePool,
    id: &str,
    created_at: i64,
    status: Status,
    initiator: Initiator,
) {
    let version = DisputeVersion {
        event_id: format!("{id}-{created_at}"),
        dispute_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at,
        status,
        initiator: Some(initiator),
        opened_at: None,
    };
    event(pool, &version.event_id, 38386, created_at).await;
    disputes_repo::insert_version(pool, &version)
        .await
        .expect("version");
    disputes_repo::refresh_projection(pool, id)
        .await
        .expect("refresh");
}

async fn settled(pool: &SqlitePool, id: &str, success_at: i64) {
    let version = OrderVersion {
        event_id: format!("{id}-{success_at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: success_at,
        direction: Direction::Buy,
        status: order::Status::Success,
        fiat_code: "ARS".to_string(),
        amount_sats: 10_000,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 0.0,
        network: Some(Network::Mainnet),
        expires_at: success_at + 86_400,
    };
    event(pool, &version.event_id, 38383, success_at).await;
    orders::insert_version(pool, &version)
        .await
        .expect("version");
    orders::refresh_projection(pool, id).await.expect("refresh");
}

/// Four settled orders and two disputes: one by the buyer, settled after
/// an hour; one by the seller, still open.
async fn seeded() -> SqlitePool {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    for (id, at) in [("o1", 100), ("o2", 200), ("o3", 300), ("o4", 400)] {
        settled(&pool, id, FROM + at).await;
    }
    dispute(&pool, "d1", FROM + 500, Status::Initiated, Initiator::Buyer).await;
    dispute(
        &pool,
        "d1",
        FROM + 500 + 3_600,
        Status::Settled,
        Initiator::Buyer,
    )
    .await;
    dispute(
        &pool,
        "d2",
        FROM + 600,
        Status::Initiated,
        Initiator::Seller,
    )
    .await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    pool
}

fn query() -> Query {
    Query {
        network_narrowed: false,
        range: Range::resolve(Some(FROM), Some(UNTIL), NOW).expect("window"),
        scope: Scope {
            pubkey: None,
            networks: vec![Network::Mainnet],
        },
    }
}

fn value<'a>(report: &'a Report, name: &str) -> &'a Value {
    &report
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the report"))
        .value
}

#[tokio::test]
async fn the_global_report_reads_the_seeded_disputes() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(&pool, &query(), None, NOW).await.expect("report");

    // Assert
    assert_eq!(value(&report, "disputes.opened"), &Value::Count(2));
    assert_eq!(value(&report, "disputes.status.settled"), &Value::Count(1));
    assert_eq!(
        value(&report, "disputes.initiator.buyer"),
        &Value::Ratio(0.5)
    );
    assert_eq!(value(&report, "disputes.rate"), &Value::Ratio(0.5));
    assert_eq!(
        value(&report, "disputes.outcome.settled"),
        &Value::Ratio(1.0)
    );
    assert_eq!(
        value(&report, "disputes.resolution_p50"),
        &Value::Seconds(3_600)
    );
    assert_eq!(value(&report, "disputes.open_now"), &Value::Count(1));
    assert_eq!(
        value(&report, "disputes.open.1.id"),
        &Value::Text("d2".into())
    );
    assert_eq!(
        value(&report, "disputes.open.1.age"),
        &Value::Seconds(NOW - (FROM + 600))
    );
}

#[tokio::test]
async fn slicing_by_instance_labels_the_slice() {
    let pool = seeded().await;

    let report = report(&pool, &query(), Some(Dimension::Instance), NOW)
        .await
        .expect("report");

    assert_eq!(
        value(&report, "disputes.Alpha (82fa8cb9).opened"),
        &Value::Count(2)
    );
}

#[tokio::test]
async fn a_network_scope_is_refused_with_the_reason() {
    let error = refuse_network_scope(Some(Network::Mainnet)).expect_err("refused");

    assert!(error.to_string().contains("no network tag"), "{error}");
    refuse_network_scope(None).expect("no scope, no objection");
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    assert_eq!(dimension(DisputeDimension::Status), Dimension::Status);
    assert_eq!(dimension(DisputeDimension::Initiator), Dimension::Initiator);
    assert_eq!(dimension(DisputeDimension::Instance), Dimension::Instance);
    assert_eq!(dimension(DisputeDimension::Period), Dimension::Month);
}

#[tokio::test]
async fn the_json_rendering_is_the_envelope_of_the_spec() {
    let pool = seeded().await;

    let rendered = report(&pool, &query(), Some(Dimension::Status), NOW)
        .await
        .expect("report")
        .render(Format::Json);
    let json: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

    assert_eq!(json["metrics"][0]["name"], "disputes.status.initiated");
    assert_eq!(json["metrics"].as_array().map(Vec::len), Some(5));
}

/// `--by day` (issue #53).
#[tokio::test]
async fn a_daily_report_names_each_day_and_keeps_the_quiet_ones() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(&pool, &query(), Some(Dimension::Day), NOW)
        .await
        .expect("report");

    // Assert: every day of the window has a block, quiet ones included.
    let days = report
        .metrics
        .iter()
        .filter(|metric| metric.name.ends_with(".opened"))
        .count();
    assert_eq!(days, 8);
    assert_eq!(
        value(&report, "disputes.2026-08-26.opened"),
        &Value::Count(0)
    );
    assert_eq!(dimension(DisputeDimension::Day), Dimension::Day);
}

#[tokio::test]
async fn a_day_the_order_history_does_not_reach_is_missing_rather_than_a_rate() {
    // Arrange: disputes reach a day further back than the orders do, and
    // `disputes.rate` divides the two. A bucket with only half of that is
    // not a rate, however confident the numerator looks.
    let pool = seeded().await;
    event(&pool, "old-dispute", 38386, FROM - 86_400).await;
    let query = Query {
        range: Range::resolve(Some(FROM - 86_400), Some(UNTIL), NOW).expect("window"),
        ..query()
    };

    // Act
    let report = report(&pool, &query, Some(Dimension::Day), NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(
        value(&report, "disputes.2026-08-24.opened"),
        &Value::Missing
    );
    assert_eq!(
        value(&report, "disputes.2026-08-25.opened"),
        &Value::Count(2),
        "the day the orders reach is answered"
    );
}

#[tokio::test]
async fn a_kind_nobody_asked_for_leaves_the_whole_report_missing() {
    // Arrange: an archive holding orders only — `backfill --kind 38383`.
    // Nothing ever looked for a dispute, so no day is a confirmed zero.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "o1", FROM + 100).await;
    crate::db::repo::indexed_kinds::record(&pool, order::KIND, 0, NOW)
        .await
        .expect("record");

    // Act
    let report = report(&pool, &query(), Some(Dimension::Day), NOW)
        .await
        .expect("report");

    // Assert
    assert!(
        report
            .metrics
            .iter()
            .filter(|metric| metric.name.ends_with(".opened"))
            .all(|metric| metric.value == Value::Missing),
        "unknown history is not zero disputes"
    );
}
