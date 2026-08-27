use std::collections::BTreeMap;

use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, rates};
use crate::ingest::parse::rates::RateSnapshot;
use crate::ingest::pipeline::seed_fixtures;
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn quoted(pool: &SqlitePool, pubkey: &str, at: i64, usd: f64) {
    let event_id = format!("rate-{pubkey}-{at}");
    events::insert_if_new(
        pool,
        &EventRecord {
            id: event_id.clone(),
            pubkey: pubkey.to_string(),
            kind: 30078,
            created_at: at,
            d_tag: Some("mostro-rates".to_string()),
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: at,
        },
    )
    .await
    .expect("event");
    rates::insert(
        pool,
        &RateSnapshot {
            event_id,
            pubkey: pubkey.to_string(),
            published_at: at,
            source: Some("yadio".to_string()),
            rates: BTreeMap::from([("USD".to_string(), usd)]),
        },
    )
    .await
    .expect("rate");
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
async fn every_known_instance_is_a_feed_even_the_one_that_never_published() {
    // Arrange: Alpha quoted twice, the latest a minute ago; Beta never.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    quoted(&pool, ALPHA, NOW - 10_000, 40_000.0).await;
    quoted(&pool, ALPHA, NOW - 60, 50_000.0).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    instances::upsert(&pool, BETA, Some("Beta"), FROM)
        .await
        .expect("beta");

    // Act
    let report = report(&pool, &query(), Some("USD"), NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(value(&report, "rates.feeds"), &Value::Count(1));
    assert_eq!(value(&report, "rates.silent"), &Value::Count(1));
    assert_eq!(
        value(&report, "rates.Alpha (82fa8cb9).age"),
        &Value::Seconds(60)
    );
    assert_eq!(
        value(&report, "rates.Alpha (82fa8cb9).status"),
        &Value::Text("fresh".into())
    );
    assert_eq!(
        value(&report, "rates.USD.Alpha (82fa8cb9)"),
        &Value::fiat(50_000.0, "USD"),
        "the latest snapshot, not the older one"
    );
    assert_eq!(
        value(&report, "rates.Beta (1b7b0f8d).status"),
        &Value::Text("silent".into())
    );
}

#[tokio::test]
async fn two_instances_quoting_at_the_same_instant_have_a_disparity() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    quoted(&pool, ALPHA, NOW - 60, 50_000.0).await;
    quoted(&pool, BETA, NOW - 70, 52_000.0).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    instances::upsert(&pool, BETA, Some("Beta"), FROM)
        .await
        .expect("beta");

    let report = report(&pool, &query(), Some("USD"), NOW)
        .await
        .expect("report");

    assert_eq!(value(&report, "rates.USD.quoted_by"), &Value::Count(2));
    assert_eq!(value(&report, "rates.USD.comparable"), &Value::Count(2));
    assert_eq!(
        value(&report, "rates.USD.low"),
        &Value::fiat(50_000.0, "USD")
    );
    assert!(matches!(
        value(&report, "rates.USD.disparity"),
        Value::Ratio(ratio) if (ratio - 0.04).abs() < 1e-9
    ));
}

#[tokio::test]
async fn the_instance_flag_narrows_the_feeds() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    quoted(&pool, ALPHA, NOW - 60, 50_000.0).await;
    quoted(&pool, BETA, NOW - 70, 52_000.0).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");
    instances::upsert(&pool, BETA, Some("Beta"), FROM)
        .await
        .expect("beta");

    let scoped = Query {
        scope: Scope {
            pubkey: Some(ALPHA.to_string()),
            ..query().scope
        },
        ..query()
    };
    let report = report(&pool, &scoped, None, NOW).await.expect("report");

    assert_eq!(value(&report, "rates.feeds"), &Value::Count(1));
    assert!(
        !report
            .metrics
            .iter()
            .any(|metric| metric.name.contains("Beta"))
    );
}

/// Over the real corpus one snapshot is stored: the other 30078 comes from
/// a pubkey never seen publishing a `y`-tagged event, and the pipeline does
/// not vouch for it. So one dead feed, eleven instances that have published
/// no rate at all, and a currency quoted by a single voice — which is no
/// disparity, and says so.
#[tokio::test]
async fn the_feeds_over_the_real_corpus_match_the_hand_count() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;

    let report = report(&pool, &query(), Some("USD"), now)
        .await
        .expect("report");

    assert_eq!(value(&report, "rates.feeds"), &Value::Count(1));
    assert_eq!(value(&report, "rates.fresh"), &Value::Count(0));
    assert_eq!(value(&report, "rates.dead"), &Value::Count(1));
    assert_eq!(value(&report, "rates.silent"), &Value::Count(11));
    assert_eq!(value(&report, "rates.USD.quoted_by"), &Value::Count(1));
    assert_eq!(value(&report, "rates.USD.comparable"), &Value::Count(1));
    assert_eq!(
        value(&report, "rates.USD.disparity"),
        &Value::Missing,
        "one voice disagrees with nobody"
    );
}

#[tokio::test]
async fn the_currency_flag_is_upper_cased_and_checked() {
    assert_eq!(currency("usd").expect("valid"), "USD");
    assert!(currency("dollars").is_err());
    assert!(currency("us$").is_err());
}

#[tokio::test]
async fn a_network_scope_is_refused_because_a_snapshot_carries_no_network() {
    let error = refuse_network_scope(Some(Network::Mainnet)).expect_err("refused");

    assert!(error.to_string().contains("30078"), "{error}");
    assert!(refuse_network_scope(None).is_ok());
}

#[tokio::test]
async fn a_feed_whose_publisher_is_not_in_the_bestiary_is_still_a_feed() {
    // A rebuild that stored a snapshot before the instance row would
    // otherwise drop the feed, and the disparity would be short a voice.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    quoted(&pool, ALPHA, NOW - 60, 50_000.0).await;

    let report = report(&pool, &query(), Some("USD"), NOW)
        .await
        .expect("report");

    assert_eq!(value(&report, "rates.feeds"), &Value::Count(1));
    assert_eq!(value(&report, "rates.USD.quoted_by"), &Value::Count(1));
    assert_eq!(
        value(&report, &format!("rates.{ALPHA}.age")),
        &Value::Seconds(60)
    );
}
