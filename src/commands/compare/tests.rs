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
async fn one_row_per_instance_from_the_seeded_database() {
    // Arrange
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a1", ALPHA, FROM + 100, 300).await;
    settled(&pool, "b1", BETA, FROM + 200, 700).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    instances::upsert(&pool, BETA, None, FROM + 1)
        .await
        .expect("beta");

    // Act
    let report = report(&pool, &query(), NOW).await.expect("report");

    // Assert
    assert_eq!(report.metrics.len(), 14);
    assert_eq!(
        value(&report, "compare.Alpha (82fa8cb9).volume_sats"),
        &Value::Sats(300)
    );
    assert_eq!(
        value(&report, &format!("compare.{BETA}.completed")),
        &Value::Count(1)
    );
}

/// One row per instance of the real corpus. Hand-counted: Fostro testing
/// completed the one `success` order (1361 sats, on regtest); Mostro Brasil
/// had one canceled and nothing completed, so its rate is 0%; Mostro's
/// four orders are all still open, so it has no rate at all.
#[tokio::test]
async fn the_comparison_over_the_real_corpus_matches_the_hand_count() {
    // Arrange
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;
    let query = Query {
        network_narrowed: false,
        range: Range::resolve(Some(1_787_500_000), Some(now), now).expect("window"),
        scope: Scope {
            pubkey: None,
            networks: vec![Network::Mainnet, Network::Regtest],
        },
    };

    // Act
    let report = report(&pool, &query, now).await.expect("report");

    // Assert
    assert_eq!(
        value(&report, "compare.Fostro testing (17b520bd).completed"),
        &Value::Count(1)
    );
    assert_eq!(
        value(&report, "compare.Fostro testing (17b520bd).volume_sats"),
        &Value::Sats(1_361)
    );
    assert_eq!(
        value(&report, "compare.Mostro Brasil (00037abd).completion_rate"),
        &Value::Ratio(0.0)
    );
    assert_eq!(
        value(&report, "compare.Mostro (82fa8cb9).completion_rate"),
        &Value::Missing
    );

    // And the table is a grid: one line per instance, not seven.
    let table = report.render_rows(Format::Table, "instance", "compare", &compare::COLUMNS);
    let fostro: Vec<&str> = table
        .lines()
        .filter(|line| line.contains("Fostro testing"))
        .collect();
    assert_eq!(fostro.len(), 1, "{table}");
    assert!(fostro[0].contains("1361 sats"), "{table}");
}

#[tokio::test]
async fn a_network_narrowed_comparison_leaves_dispute_rates_missing() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a1", ALPHA, FROM + 100, 300).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    let query = Query {
        network_narrowed: true,
        ..query()
    };

    let report = report(&pool, &query, NOW).await.expect("report");

    assert_eq!(
        value(&report, "compare.Alpha (82fa8cb9).dispute_rate"),
        &Value::Missing
    );
}
