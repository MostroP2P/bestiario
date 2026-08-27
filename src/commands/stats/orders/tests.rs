//! The command end to end below the print: a seeded database in, a
//! [`Report`] out. The arithmetic itself is tested in the stats crate; what
//! is checked here is that the wiring reads the right rows and names the
//! right things.

use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, orders};
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network;
use crate::stats::Value;

const MEMORY: &str = "sqlite::memory:";
const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";

/// The window is `[FROM, UNTIL)`; `NOW` is a day past its end.
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn version(order_id: &str, pubkey: &str, created_at: i64, status: Status) -> OrderVersion {
    OrderVersion {
        event_id: format!("{order_id}-{created_at}"),
        order_id: order_id.to_string(),
        pubkey: pubkey.to_string(),
        created_at,
        direction: Direction::Buy,
        status,
        fiat_code: "ARS".to_string(),
        amount_sats: 10_000,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 0.0,
        network: Some(Network::Mainnet),
        expires_at: created_at + 86_400,
    }
}

async fn ingest(pool: &SqlitePool, version: &OrderVersion) {
    let record = EventRecord {
        id: version.event_id.clone(),
        pubkey: version.pubkey.clone(),
        kind: 38383,
        created_at: version.created_at,
        d_tag: Some(version.order_id.clone()),
        raw_json: "{}".to_string(),
        relay_url: "wss://relay.mostro.network".to_string(),
        seen_at: version.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    orders::insert_version(pool, version)
        .await
        .expect("version");
    orders::refresh_projection(pool, &version.order_id)
        .await
        .expect("refresh");
}

/// Two instances, three orders in the window: Alpha completes one and
/// abandons one, Beta leaves one pending that is still live at `NOW`.
async fn seeded() -> SqlitePool {
    let pool = migrated().await;

    ingest(&pool, &version("done", ALPHA, FROM + 100, Status::Pending)).await;
    ingest(
        &pool,
        &version("done", ALPHA, FROM + 200, Status::InProgress),
    )
    .await;
    ingest(&pool, &version("done", ALPHA, FROM + 300, Status::Success)).await;

    ingest(&pool, &version("gone", ALPHA, FROM + 400, Status::Pending)).await;
    ingest(&pool, &version("gone", ALPHA, FROM + 500, Status::Canceled)).await;

    ingest(
        &pool,
        &OrderVersion {
            expires_at: NOW + 3_600,
            ..version("open", BETA, FROM + 600, Status::Pending)
        },
    )
    .await;

    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    instances::upsert(&pool, BETA, None, FROM)
        .await
        .expect("beta");

    pool
}

fn query(pubkey: Option<&str>) -> Query {
    Query {
        range: Range::resolve(Some(FROM), Some(UNTIL), NOW).expect("window"),
        scope: Scope {
            pubkey: pubkey.map(str::to_string),
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
async fn the_global_report_counts_the_whole_scope() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(&pool, &query(None), None, NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(value(&report, "orders.created"), &Value::Count(3));
    assert_eq!(value(&report, "orders.completed"), &Value::Count(1));
    assert_eq!(value(&report, "orders.canceled"), &Value::Count(1));
    assert_eq!(value(&report, "orders.completion_rate"), &Value::Ratio(0.5));
    assert_eq!(
        value(&report, "orders.abandonment_rate"),
        &Value::Ratio(1.0 / 3.0)
    );
    assert_eq!(value(&report, "orders.open_now"), &Value::Count(1));
    assert_eq!(value(&report, "orders.in_progress_now"), &Value::Count(0));
}

#[tokio::test]
async fn the_report_carries_the_window_it_was_asked_for() {
    let pool = seeded().await;

    let report = report(&pool, &query(None), None, NOW)
        .await
        .expect("report");

    let (from, until) = query(None).range.to_rfc3339();
    assert_eq!(report.range.from, from);
    assert_eq!(report.range.until, until);
}

#[tokio::test]
async fn an_instance_scope_leaves_the_other_instances_out() {
    let pool = seeded().await;

    let report = report(&pool, &query(Some(BETA)), None, NOW)
        .await
        .expect("report");

    assert_eq!(value(&report, "orders.created"), &Value::Count(1));
    assert_eq!(value(&report, "orders.completed"), &Value::Count(0));
}

#[tokio::test]
async fn slicing_by_instance_labels_a_named_instance_by_name_and_short_pubkey_and_a_nameless_one_by_pubkey()
 {
    let pool = seeded().await;

    let report = report(&pool, &query(None), Some(Dimension::Instance), NOW)
        .await
        .expect("report");

    assert_eq!(
        value(&report, "orders.Alpha (82fa8cb9).created"),
        &Value::Count(2)
    );
    assert_eq!(
        value(&report, &format!("orders.{BETA}.created")),
        &Value::Count(1)
    );
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    // `period` is the one that does not share a name; the rest are checked
    // so that adding a variant to either enum fails here rather than at
    // the user's terminal.
    assert_eq!(dimension(OrderDimension::Period), Dimension::Month);
    assert_eq!(dimension(OrderDimension::Status), Dimension::Status);
    assert_eq!(dimension(OrderDimension::Kind), Dimension::Kind);
    assert_eq!(dimension(OrderDimension::Fiat), Dimension::Fiat);
    assert_eq!(dimension(OrderDimension::Method), Dimension::Method);
    assert_eq!(dimension(OrderDimension::Instance), Dimension::Instance);
    assert_eq!(dimension(OrderDimension::Hour), Dimension::Hour);
    assert_eq!(dimension(OrderDimension::Weekday), Dimension::Weekday);
}

#[tokio::test]
async fn the_json_rendering_is_the_envelope_of_the_spec() {
    let pool = seeded().await;

    let rendered = report(&pool, &query(None), None, NOW)
        .await
        .expect("report")
        .render(Format::Json);
    let json: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

    assert!(json["generated_at"].is_string());
    assert!(json["range"]["from"].is_string());
    assert_eq!(json["metrics"][0]["name"], "orders.created");
    assert_eq!(json["metrics"][0]["kind"], "observed");
}
