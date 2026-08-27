use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, orders};
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network as Net;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
/// 2026-07-01, 2026-08-01 and 2026-09-01 at midnight UTC.
const JULY: i64 = 1_782_864_000;
const AUGUST: i64 = 1_785_542_400;
const SEPTEMBER: i64 = 1_788_220_800;
const DAY: i64 = 86_400;

async fn version(pool: &SqlitePool, id: &str, at: i64, status: Status, fiat: &str, sats: i64) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status,
        fiat_code: fiat.to_string(),
        amount_sats: sats,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 0.0,
        network: Some(Net::Mainnet),
        expires_at: at + DAY,
    };
    events::insert_if_new(
        pool,
        &EventRecord {
            id: version.event_id.clone(),
            pubkey: ALPHA.to_string(),
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

/// One order completed in July (5 000 sats, ARS) and two in August
/// (10 000 ARS, 30 000 USD).
async fn seeded() -> SqlitePool {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    for (id, created, fiat, sats) in [
        ("j1", JULY + DAY, "ARS", 5_000),
        ("a1", AUGUST + DAY, "ARS", 10_000),
        ("a2", AUGUST + 3 * DAY, "USD", 30_000),
    ] {
        version(&pool, id, created, Status::Pending, fiat, sats).await;
        version(&pool, id, created + DAY, Status::Success, fiat, sats).await;
    }
    instances::upsert(&pool, ALPHA, Some("Alpha"), JULY)
        .await
        .expect("alpha");
    pool
}

fn query() -> Query {
    Query {
        network_narrowed: false,
        range: Range::resolve(Some(JULY), Some(SEPTEMBER), SEPTEMBER).expect("window"),
        scope: Scope {
            pubkey: None,
            networks: vec![Net::Mainnet],
        },
    }
}

fn assumptions() -> AssumptionSettings {
    AssumptionSettings::default()
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
async fn a_monthly_series_reads_the_seeded_orders() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(
        &pool,
        &query(),
        &assumptions(),
        "volume.sats",
        Period::Month,
        None,
        SEPTEMBER,
    )
    .await
    .expect("series");

    // Assert
    assert_eq!(value(&report, "volume.sats.2026-07"), &Value::Sats(5_000));
    assert_eq!(value(&report, "volume.sats.2026-08"), &Value::Sats(40_000));
    assert_eq!(value(&report, "volume.sats.2026-07.delta"), &Value::Missing);
}

#[tokio::test]
async fn a_split_series_labels_each_line() {
    let pool = seeded().await;

    let report = report(
        &pool,
        &query(),
        &assumptions(),
        "volume.sats",
        Period::Month,
        Some(Split::Fiat),
        SEPTEMBER,
    )
    .await
    .expect("series");

    assert_eq!(
        value(&report, "volume.sats.USD.2026-08"),
        &Value::Sats(30_000)
    );
    assert_eq!(
        value(&report, "volume.sats.ARS.2026-07"),
        &Value::Sats(5_000)
    );
}

#[tokio::test]
async fn an_unknown_metric_is_answered_with_the_ones_that_exist() {
    let pool = seeded().await;

    let error = report(
        &pool,
        &query(),
        &assumptions(),
        "volume.nonsense",
        Period::Month,
        None,
        SEPTEMBER,
    )
    .await
    .expect_err("refused");

    let message = error.to_string();
    assert!(message.contains("volume.nonsense"), "{message}");
    assert!(message.contains("volume.sats"), "{message}");
    assert!(message.contains("orders.created"), "{message}");
}

#[tokio::test]
async fn a_series_reads_only_the_family_it_plots() {
    // Disputes were never loaded for a volume series, so a disputes metric
    // over the same archive is still right — the load is per family, not a
    // cache shared between them.
    let pool = seeded().await;

    let volume = report(
        &pool,
        &query(),
        &assumptions(),
        "volume.sats",
        Period::Month,
        None,
        SEPTEMBER,
    )
    .await
    .expect("series");
    let disputes = report(
        &pool,
        &query(),
        &assumptions(),
        "disputes.opened",
        Period::Month,
        None,
        SEPTEMBER,
    )
    .await
    .expect("series");

    assert_eq!(value(&volume, "volume.sats.2026-08"), &Value::Sats(40_000));
    assert_eq!(
        value(&disputes, "disputes.opened.2026-08"),
        &Value::Count(0)
    );
}

#[tokio::test]
async fn the_inferred_dev_fee_rows_are_plottable_because_the_assumption_travels() {
    let pool = seeded().await;

    let report = report(
        &pool,
        &query(),
        &assumptions(),
        "dev_fees.implied_volume",
        Period::Month,
        None,
        SEPTEMBER,
    )
    .await
    .expect("series");

    assert_eq!(
        value(&report, "dev_fees.implied_volume.2026-08"),
        &Value::Sats(0),
        "no fees seeded, but the metric exists and is plotted"
    );
}

#[tokio::test]
async fn every_cli_period_and_split_maps_to_an_aggregation_one() {
    assert_eq!(period(CliPeriod::Day), Period::Day);
    assert_eq!(period(CliPeriod::Week), Period::Week);
    assert_eq!(period(CliPeriod::Month), Period::Month);
    assert_eq!(period(CliPeriod::Year), Period::Year);
    assert_eq!(dimension(SeriesSplit::Instance), Split::Instance);
    assert_eq!(dimension(SeriesSplit::Kind), Split::Kind);
    assert_eq!(dimension(SeriesSplit::Fiat), Split::Fiat);
}
