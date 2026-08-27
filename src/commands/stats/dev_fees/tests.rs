//! The command below the print: a seeded database in, a [`Report`] out.

use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::config::AssumptionSettings;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{dev_fees as fees_repo, instance_info, instances, orders};
use crate::ingest::parse::dev_fee::DevFee;
use crate::ingest::parse::info::InstanceInfo;
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
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

async fn settled(pool: &SqlitePool, order_id: &str, success_at: i64) {
    let version = OrderVersion {
        event_id: format!("{order_id}-{success_at}"),
        order_id: order_id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: success_at,
        direction: Direction::Buy,
        status: Status::Success,
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
    orders::refresh_projection(pool, order_id)
        .await
        .expect("refresh");
}

async fn fee(pool: &SqlitePool, id: &str, order_id: &str, created_at: i64, amount_sats: i64) {
    event(pool, id, 8383, created_at).await;
    fees_repo::insert(
        pool,
        &DevFee {
            event_id: id.to_string(),
            pubkey: ALPHA.to_string(),
            order_id: order_id.to_string(),
            created_at,
            amount_sats,
            payment_hash: format!("hash-{id}"),
            destination: None,
            network: Some(Network::Mainnet),
        },
    )
    .await
    .expect("fee");
}

/// Two settled orders, one paid for (300 sats, 60s late), one not; plus an
/// orphan fee of 50 sats. The instance published a fee above zero before
/// either settled, so both count towards coverage.
async fn seeded() -> SqlitePool {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    event(&pool, "info", 38385, FROM).await;
    instance_info::insert_version(
        &pool,
        &InstanceInfo {
            event_id: "info".to_string(),
            pubkey: ALPHA.to_string(),
            created_at: FROM,
            fee: Some(0.006),
            max_order_amount: None,
            min_order_amount: None,
            fiat_currencies: None,
            mostro_version: None,
            protocol_version: None,
            ln_networks: None,
            bond_enabled: None,
        },
    )
    .await
    .expect("info");
    settled(&pool, "paid", FROM + 100).await;
    settled(&pool, "unpaid", FROM + 200).await;
    fee(&pool, "f1", "paid", FROM + 160, 300).await;
    fee(&pool, "f2", "unseen", FROM + 300, 50).await;
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
async fn the_global_report_reads_the_seeded_fees() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(&pool, &query(), None, &AssumptionSettings::default(), NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(value(&report, "dev_fees.total_sats"), &Value::Sats(350));
    assert_eq!(value(&report, "dev_fees.paid"), &Value::Count(2));
    assert_eq!(value(&report, "dev_fees.coverage"), &Value::Ratio(0.5));
    assert_eq!(value(&report, "dev_fees.latency_p50"), &Value::Seconds(60));
    assert_eq!(value(&report, "dev_fees.orphans"), &Value::Count(1));
    assert_eq!(value(&report, "dev_fees.duplicates"), &Value::Count(0));
}

#[tokio::test]
async fn slicing_by_instance_labels_the_slice() {
    let pool = seeded().await;

    let report = report(
        &pool,
        &query(),
        Some(Dimension::Instance),
        &AssumptionSettings::default(),
        NOW,
    )
    .await
    .expect("report");

    assert_eq!(
        value(&report, "dev_fees.Alpha (82fa8cb9).total_sats"),
        &Value::Sats(350)
    );
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    assert_eq!(dimension(InstanceOrPeriod::Instance), Dimension::Instance);
    assert_eq!(dimension(InstanceOrPeriod::Period), Dimension::Month);
}

#[tokio::test]
async fn the_json_rendering_is_the_envelope_of_the_spec() {
    let pool = seeded().await;

    let rendered = report(&pool, &query(), None, &AssumptionSettings::default(), NOW)
        .await
        .expect("report")
        .render(Format::Json);
    let json: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

    assert_eq!(json["metrics"][0]["name"], "dev_fees.total_sats");
    assert_eq!(json["metrics"][0]["unit"], "sats");
}

#[tokio::test]
async fn the_implied_volume_rests_on_the_configured_share() {
    // Arrange: the seed's two fees, 300 + 50 sats, under fee 0.006. At the
    // default share of 0.30 that is 350 / 0.0018 = 194_444 sats; at a
    // configured 0.60 it is half.
    let pool = seeded().await;
    let mut assumptions = AssumptionSettings::default();
    assumptions
        .dev_fee_percentage
        .insert(ALPHA.to_string(), 0.60);

    // Act
    let default = report(&pool, &query(), None, &AssumptionSettings::default(), NOW)
        .await
        .expect("report");
    let doubled = report(&pool, &query(), None, &assumptions, NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(
        value(&default, "dev_fees.implied_volume"),
        &Value::Sats(194_444)
    );
    assert_eq!(
        value(&doubled, "dev_fees.implied_volume"),
        &Value::Sats(97_222)
    );
    // Only the paid order is known: 10_000 sats observed.
    assert_eq!(
        value(&default, "dev_fees.with_fee_volume"),
        &Value::Sats(10_000)
    );
    let implied = default
        .metrics
        .iter()
        .find(|metric| metric.name == "dev_fees.implied_volume")
        .expect("present");
    assert!(implied.is_inferred());
    assert!(implied.error().expect("error").contains("pct assumed 0.30"));
}

/// `--by day` (issue #53). Both seeded fees are paid on 2026-08-25.
#[tokio::test]
async fn a_daily_report_names_each_day_and_keeps_the_quiet_ones() {
    // Arrange
    let pool = seeded().await;

    // Act
    let report = report(
        &pool,
        &query(),
        Some(Dimension::Day),
        &AssumptionSettings::default(),
        NOW,
    )
    .await
    .expect("report");

    // Assert
    assert_eq!(
        value(&report, "dev_fees.2026-08-25.total_sats"),
        &Value::Sats(350)
    );
    assert_eq!(
        value(&report, "dev_fees.2026-08-26.total_sats"),
        &Value::Sats(0),
        "a day with no fee is zero, not absent"
    );
    assert_eq!(dimension(InstanceOrPeriod::Day), Dimension::Day);
}
