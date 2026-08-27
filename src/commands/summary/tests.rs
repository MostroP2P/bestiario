use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::orders;
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn settled(pool: &SqlitePool, id: &str, at: i64, fiat: &str, sats: i64) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status: Status::Success,
        fiat_code: fiat.to_string(),
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

fn query() -> Query {
    Query {
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
async fn the_summary_reads_the_seeded_orders() {
    // Arrange
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "o1", FROM + 100, "ARS", 300).await;
    settled(&pool, "o2", FROM + 200, "VES", 700).await;

    // Act
    let report = report(&pool, &query(), NOW).await.expect("report");

    // Assert
    assert_eq!(value(&report, "summary.created"), &Value::Count(2));
    assert_eq!(value(&report, "summary.volume_sats"), &Value::Sats(1_000));
    assert_eq!(value(&report, "summary.active_instances"), &Value::Count(1));
    assert_eq!(
        value(&report, "summary.top_fiat"),
        &Value::Text("ARS (1), VES (1)".into())
    );
    assert_eq!(value(&report, "summary.open_disputes"), &Value::Count(0));
}

#[tokio::test]
async fn an_empty_database_still_produces_the_whole_view() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");

    let report = report(&pool, &query(), NOW).await.expect("report");

    assert_eq!(report.metrics.len(), 8);
    assert_eq!(value(&report, "summary.top_methods"), &Value::Missing);
}
