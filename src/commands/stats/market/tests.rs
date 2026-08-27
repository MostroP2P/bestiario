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

async fn version(pool: &SqlitePool, id: &str, at: i64, status: Status, direction: Direction) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction,
        status,
        fiat_code: "ARS".to_string(),
        amount_sats: 10_000,
        fiat: FiatAmount::Fixed(50.0),
        payment_methods: vec!["cash".to_string()],
        premium: 3.0,
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
async fn the_global_report_reads_the_seeded_orders() {
    // Arrange: two buys and a sell created in the window, one buy completed.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, "a", FROM + 100, Status::Pending, Direction::Buy).await;
    version(&pool, "a", FROM + 200, Status::Success, Direction::Buy).await;
    version(&pool, "b", FROM + 300, Status::Pending, Direction::Buy).await;
    version(&pool, "c", FROM + 400, Status::Pending, Direction::Sell).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    // Act
    let report = report(&pool, &query(), None, NOW).await.expect("report");

    // Assert
    assert_eq!(value(&report, "market.orders"), &Value::Count(3));
    assert!(matches!(
        value(&report, "market.buy_orders_share"),
        Value::Ratio(share) if (share - 2.0 / 3.0).abs() < 1e-9
    ));
    assert_eq!(
        value(&report, "market.buy_volume_share"),
        &Value::Ratio(1.0)
    );
    assert_eq!(value(&report, "market.premium_p50"), &Value::Ratio(0.03));
    assert_eq!(
        value(&report, "market.fiat_top3_by_orders"),
        &Value::Text("ARS 3".into())
    );
    assert_eq!(
        value(&report, "market.new_fiats"),
        &Value::Text("ARS".into())
    );
}

#[tokio::test]
async fn slicing_by_fiat_labels_the_slice_and_drops_the_fiat_ranking() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, "a", FROM + 100, Status::Pending, Direction::Buy).await;

    let report = report(&pool, &query(), Some(Dimension::Fiat), NOW)
        .await
        .expect("report");

    assert_eq!(value(&report, "market.ARS.orders"), &Value::Count(1));
    assert!(
        !report
            .metrics
            .iter()
            .any(|metric| metric.name.contains("fiat_top3"))
    );
}

/// Over the real corpus: eight Mostro orders created in the window, four
/// of them ranges (`fa = [min, max]`) and four born at market price
/// (`amt = 0`), and every currency new since the corpus starts inside the
/// window.
#[tokio::test]
async fn the_market_over_the_real_corpus_matches_the_hand_count() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;
    let query = Query {
        range: Range::resolve(Some(1_787_500_000), Some(now), now).expect("window"),
        ..query()
    };

    let report = report(&pool, &query, None, now).await.expect("report");

    assert_eq!(value(&report, "market.orders"), &Value::Count(8));
    assert!(matches!(
        value(&report, "market.range_share"),
        Value::Ratio(share) if (share - 0.5).abs() < 1e-9
    ));
    assert!(matches!(
        value(&report, "market.market_price_share"),
        Value::Ratio(share) if (share - 0.5).abs() < 1e-9
    ));
    assert!(matches!(value(&report, "market.new_fiats"), Value::Text(_)));
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    assert_eq!(dimension(MarketDimension::Fiat), Dimension::Fiat);
    assert_eq!(dimension(MarketDimension::Kind), Dimension::Kind);
    assert_eq!(dimension(MarketDimension::Instance), Dimension::Instance);
}
