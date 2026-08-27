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

async fn version(pool: &SqlitePool, id: &str, at: i64, status: Status) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status,
        fiat_code: "ARS".to_string(),
        amount_sats: 10_000,
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
async fn the_global_report_reads_the_versions_of_the_seeded_orders() {
    // Arrange: one order pending → in-progress (120s) → success (300s more);
    // one pending → canceled untaken (600s); one still pending.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, "a", FROM + 100, Status::Pending).await;
    version(&pool, "a", FROM + 220, Status::InProgress).await;
    version(&pool, "a", FROM + 520, Status::Success).await;
    version(&pool, "b", FROM + 1_000, Status::Pending).await;
    version(&pool, "b", FROM + 1_600, Status::Canceled).await;
    version(&pool, "c", FROM + 2_000, Status::Pending).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    // Act
    let report = report(&pool, &query(), None, NOW).await.expect("report");

    // Assert
    assert_eq!(
        value(&report, "timing.time_to_fill_p50"),
        &Value::Seconds(120)
    );
    assert_eq!(
        value(&report, "timing.time_to_complete_p50"),
        &Value::Seconds(300)
    );
    assert_eq!(
        value(&report, "timing.full_cycle_p50"),
        &Value::Seconds(420)
    );
    assert_eq!(
        value(&report, "timing.time_to_cancel_p50"),
        &Value::Seconds(600)
    );
    assert_eq!(value(&report, "timing.funnel.created"), &Value::Count(3));
    assert_eq!(value(&report, "timing.funnel.taken"), &Value::Count(1));
    assert_eq!(
        value(&report, "timing.funnel.canceled_untaken"),
        &Value::Count(1)
    );
    // `c` expired a day after FROM + 2000, long before NOW: not live, and
    // with no terminal version seen it is expired, not open.
    assert_eq!(value(&report, "timing.book_size"), &Value::Count(0));
    assert_eq!(
        value(&report, "timing.funnel.expired_untaken"),
        &Value::Count(1)
    );
    assert_eq!(value(&report, "timing.funnel.open"), &Value::Count(0));
}

#[tokio::test]
async fn slicing_by_method_labels_the_slice() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    version(&pool, "a", FROM + 100, Status::Pending).await;
    version(&pool, "a", FROM + 220, Status::InProgress).await;

    let report = report(&pool, &query(), Some(Dimension::Method), NOW)
        .await
        .expect("report");

    assert_eq!(
        value(&report, "timing.cash.time_to_fill_samples"),
        &Value::Count(1)
    );
}

/// Over the real corpus every order has a single version, so no gap can be
/// measured; the ones seen at `pending` form the cohort, the rest are of
/// unknown origin.
#[tokio::test]
async fn the_timing_over_the_real_corpus_matches_the_hand_count() {
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

    eprintln!(
        "PEEK created={:?} unknown={:?} completed={:?} open={:?} expired={:?} book={:?}",
        value(&report, "timing.funnel.created"),
        value(&report, "timing.unknown_origin"),
        value(&report, "timing.funnel.completed"),
        value(&report, "timing.funnel.open"),
        value(&report, "timing.funnel.expired_untaken"),
        value(&report, "timing.book_size")
    );
    assert_eq!(value(&report, "timing.time_to_fill_p50"), &Value::Missing);
    assert_eq!(value(&report, "timing.funnel.completed"), &Value::Count(0));
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    assert_eq!(dimension(TimingDimension::Fiat), Dimension::Fiat);
    assert_eq!(dimension(TimingDimension::Method), Dimension::Method);
    assert_eq!(dimension(TimingDimension::Kind), Dimension::Kind);
    assert_eq!(dimension(TimingDimension::Instance), Dimension::Instance);
}
