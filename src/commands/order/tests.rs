use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::ingest::parse::dev_fee::DevFee;
use crate::ingest::parse::order::{Direction, OrderVersion, Status};
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const T0: i64 = 1_787_700_000;
const NOW: i64 = T0 + 86_400;

async fn event(pool: &SqlitePool, id: &str, kind: u16, at: i64) {
    events::insert_if_new(
        pool,
        &EventRecord {
            id: id.to_string(),
            pubkey: ALPHA.to_string(),
            kind: kind.into(),
            created_at: at,
            d_tag: None,
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: at,
        },
    )
    .await
    .expect("event");
}

async fn version(pool: &SqlitePool, at: i64, status: Status) {
    let version = OrderVersion {
        event_id: format!("v-{at}"),
        order_id: "o1".to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Sell,
        status,
        fiat_code: "VES".to_string(),
        amount_sats: 21_000,
        fiat: FiatAmount::Fixed(100.0),
        payment_methods: vec!["cash".to_string()],
        premium: 5.0,
        network: Some(Network::Mainnet),
        expires_at: at + 900,
    };
    event(pool, &version.event_id, 38383, at).await;
    orders::insert_version(pool, &version)
        .await
        .expect("version");
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
async fn the_lifecycle_lists_every_version_oldest_first_and_the_fee() {
    // Arrange: versions stored newest first, as a backfill would.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, T0 + 200, Status::Success).await;
    version(&pool, T0, Status::Pending).await;
    version(&pool, T0 + 100, Status::InProgress).await;
    event(&pool, "fee", 8383, T0 + 300).await;
    dev_fees::insert(
        &pool,
        &DevFee {
            event_id: "fee".to_string(),
            pubkey: ALPHA.to_string(),
            order_id: "o1".to_string(),
            created_at: T0 + 300,
            amount_sats: 63,
            payment_hash: "hash".to_string(),
            destination: None,
            network: Some(Network::Mainnet),
        },
    )
    .await
    .expect("fee");

    // Act
    let report = report(&pool, "o1", NOW).await.expect("report");

    // Assert
    assert_eq!(value(&report, "order.versions"), &Value::Count(3));
    assert_eq!(
        value(&report, "order.1.status"),
        &Value::Text("pending".into())
    );
    assert_eq!(
        value(&report, "order.2.status"),
        &Value::Text("in-progress".into())
    );
    assert_eq!(
        value(&report, "order.3.status"),
        &Value::Text("success".into())
    );
    assert_eq!(value(&report, "dev_fee.1.amount"), &Value::Sats(63));
    // The range is the order's own span: first version to one past the last.
    assert_eq!(report.range.from, "2026-08-25T23:20:00+00:00");
    assert_eq!(report.range.until, "2026-08-25T23:23:21+00:00");
}

#[tokio::test]
async fn an_unknown_order_is_an_error_not_an_empty_report() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");

    let error = report(&pool, "nope", NOW).await.expect_err("unknown");

    assert!(error.to_string().contains("nope"), "{error}");
}
