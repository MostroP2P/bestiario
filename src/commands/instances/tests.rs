//! Both commands below the print: a seeded database in, a [`Report`] out.

use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances as instances_repo, orders};
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn settled(pool: &SqlitePool, id: &str, pubkey: &str, at: i64, sats: i64) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: pubkey.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status: Status::Success,
        fiat_code: "ARS".to_string(),
        amount_sats: sats,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 0.0,
        network: Some(Network::Mainnet),
        expires_at: at + 86_400,
    };
    events::insert_if_new(
        pool,
        &EventRecord {
            id: version.event_id.clone(),
            pubkey: pubkey.to_string(),
            kind: 38383,
            created_at: at,
            d_tag: Some(id.to_string()),
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: at,
        },
    )
    .await
    .expect("event");
    orders::insert_version(pool, &version)
        .await
        .expect("version");
    orders::refresh_projection(pool, id).await.expect("refresh");
}

/// Alpha settled 300 sats in one order, Beta 700 in one; Beta went quiet
/// long ago.
async fn seeded() -> SqlitePool {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a1", ALPHA, FROM + 100, 300).await;
    settled(&pool, "b1", BETA, FROM + 200, 700).await;
    instances_repo::upsert(&pool, ALPHA, Some("Alpha"), NOW - 60)
        .await
        .expect("alpha");
    instances_repo::upsert(&pool, BETA, None, FROM - 30 * 86_400)
        .await
        .expect("beta");
    pool
}

fn query(pubkey: Option<&str>) -> Query {
    Query {
        network_narrowed: false,
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
async fn the_list_has_every_instance_with_its_activity_and_silence() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = list_report(&pool, &query(None), NOW).await.expect("report");

    // Assert
    assert_eq!(
        value(&report, "instances.Alpha (82fa8cb9).created"),
        &Value::Count(1)
    );
    assert_eq!(
        value(&report, "instances.Alpha (82fa8cb9).silent"),
        &Value::Text("no".into())
    );
    assert_eq!(
        value(&report, &format!("instances.{BETA}.silent")),
        &Value::Text("yes".into())
    );
}

#[tokio::test]
async fn the_profile_reports_the_instance_and_its_share() {
    let pool = seeded().await;

    let report = profile_report(&pool, &query(Some(ALPHA)), NOW)
        .await
        .expect("report");

    assert_eq!(
        value(&report, "instance.pubkey"),
        &Value::Text(ALPHA.into())
    );
    assert_eq!(value(&report, "orders.completed"), &Value::Count(1));
    assert_eq!(value(&report, "volume.sats"), &Value::Sats(300));
    assert_eq!(value(&report, "share.orders"), &Value::Ratio(0.5));
    assert_eq!(value(&report, "share.volume"), &Value::Ratio(0.3));
}

#[tokio::test]
async fn a_network_narrowed_profile_reports_disputes_as_missing() {
    let pool = seeded().await;
    let query = Query {
        network_narrowed: true,
        ..query(Some(ALPHA))
    };

    let report = profile_report(&pool, &query, NOW).await.expect("report");

    assert_eq!(value(&report, "disputes.opened"), &Value::Missing);
    assert_eq!(value(&report, "orders.completed"), &Value::Count(1));
}

#[tokio::test]
async fn a_profile_needs_an_instance() {
    let pool = seeded().await;

    let error = profile_report(&pool, &query(None), NOW)
        .await
        .expect_err("no instance");

    assert!(error.to_string().contains("instance"), "{error}");
}

#[tokio::test]
async fn the_list_renders_as_a_table_with_one_row_per_field() {
    let pool = seeded().await;

    let table = list_report(&pool, &query(None), NOW)
        .await
        .expect("report")
        .render(Format::Table);

    assert!(table.contains("instances.Alpha (82fa8cb9).name"), "{table}");
    assert!(table.contains("Alpha"), "{table}");
}
