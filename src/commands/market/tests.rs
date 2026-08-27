use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, orders};
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::ingest::pipeline::seed_fixtures;
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn version(pool: &SqlitePool, id: &str, at: i64, status: Status, fiat: &str) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status,
        fiat_code: fiat.to_string(),
        amount_sats: 10_000,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 2.0,
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
        network_narrowed: false,
        range: Range::resolve(Some(FROM), Some(UNTIL), NOW).expect("window"),
        scope: Scope {
            pubkey: None,
            networks: vec![Network::Mainnet, Network::Regtest],
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
async fn the_view_covers_one_currency_and_not_the_others() {
    // Arrange: two ARS orders, one of them taken, and one USD order.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, "a", FROM + 100, Status::Pending, "ARS").await;
    version(&pool, "a", FROM + 200, Status::InProgress, "ARS").await;
    version(&pool, "b", FROM + 300, Status::Pending, "ARS").await;
    version(&pool, "u", FROM + 400, Status::Pending, "USD").await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    // Act
    let report = report(&pool, &query(), "ARS", NOW).await.expect("report");

    // Assert
    assert_eq!(value(&report, "market.ARS.orders"), &Value::Count(2));
    assert_eq!(
        value(&report, "market.ARS.time_to_fill_p50"),
        &Value::Seconds(100)
    );
    assert_eq!(
        value(&report, "market.ARS.instances_top3_by_orders"),
        &Value::Text("Alpha (82fa8cb9) 2".into())
    );
    assert!(
        !report
            .metrics
            .iter()
            .any(|metric| metric.name.starts_with("market.USD"))
    );
}

/// Over the real corpus: two orders stand in ARS, published by one
/// instance, neither of them taken.
#[tokio::test]
async fn the_view_over_the_real_corpus_matches_the_hand_count() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;
    let query = Query {
        range: Range::resolve(Some(1_787_500_000), Some(now), now).expect("window"),
        ..query()
    };

    let report = report(&pool, &query, "ARS", now).await.expect("report");

    assert_eq!(value(&report, "market.ARS.orders"), &Value::Count(2));
    assert_eq!(
        value(&report, "market.ARS.time_to_fill_samples"),
        &Value::Count(0)
    );
    assert!(matches!(
        value(&report, "market.ARS.instances_top3_by_orders"),
        Value::Text(_)
    ));
}

#[tokio::test]
async fn the_argument_is_upper_cased_and_checked() {
    assert_eq!(currency("ars").expect("valid"), "ARS");
    assert_eq!(currency("USD").expect("valid"), "USD");
    assert!(currency("pesos").is_err());
    assert!(currency("ar$").is_err());
    assert!(currency("").is_err());
}
