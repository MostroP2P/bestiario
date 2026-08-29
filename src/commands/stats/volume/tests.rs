use sqlx::SqlitePool;

use super::*;
use crate::commands::range::Range;
use crate::db::connect_and_migrate;
use crate::db::load::Scope;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, orders, rates};
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::ingest::parse::rates::RateSnapshot;
use crate::ingest::pipeline::seed_fixtures;
use crate::network::Network;
use crate::stats::Value;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = FROM + 7 * 86_400;
const NOW: i64 = UNTIL + 86_400;

async fn settled(pool: &SqlitePool, id: &str, at: i64, sats: i64, fiat: FiatAmount) {
    let version = OrderVersion {
        event_id: format!("{id}-{at}"),
        order_id: id.to_string(),
        pubkey: ALPHA.to_string(),
        created_at: at,
        direction: Direction::Buy,
        status: Status::Success,
        fiat_code: "ARS".to_string(),
        amount_sats: sats,
        fiat,
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
async fn the_global_report_reads_the_seeded_orders() {
    // Arrange: two fixed-amount orders and a range one.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    settled(&pool, "b", FROM + 200, 30_000, FiatAmount::Fixed(300.0)).await;
    settled(
        &pool,
        "c",
        FROM + 300,
        2_000_000,
        FiatAmount::Range {
            min: 10.0,
            max: 100.0,
        },
    )
    .await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    // Act
    let report = report(&pool, &query(), None, None, NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(value(&report, "volume.sats"), &Value::Sats(2_035_000));
    assert_eq!(value(&report, "volume.completed"), &Value::Count(3));
    assert_eq!(value(&report, "volume.size.gt_1m"), &Value::Count(1));
    assert_eq!(
        value(&report, "volume.fiat.ARS.total"),
        &Value::Fiat {
            amount: 350.0,
            code: "ARS".into()
        }
    );
    assert_eq!(value(&report, "volume.fiat.ARS.orders"), &Value::Count(2));
}

#[tokio::test]
async fn slicing_by_instance_labels_the_slice() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    let report = report(&pool, &query(), Some(Dimension::Instance), None, NOW)
        .await
        .expect("report");

    assert_eq!(
        value(&report, "volume.Alpha (82fa8cb9).sats"),
        &Value::Sats(5_000)
    );
}

/// Over the real corpus: one completed order, Fostro testing's 1361 sats
/// for 21500 CUP, in the 10k–50k bucket... no: 1361 sats is under 10k.
#[tokio::test]
async fn the_volume_over_the_real_corpus_matches_the_hand_count() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;
    let query = Query {
        range: Range::resolve(Some(1_787_500_000), Some(now), now).expect("window"),
        ..query()
    };

    let report = report(&pool, &query, None, None, now)
        .await
        .expect("report");

    assert_eq!(value(&report, "volume.completed"), &Value::Count(1));
    assert_eq!(value(&report, "volume.sats"), &Value::Sats(1_361));
    assert_eq!(value(&report, "volume.largest"), &Value::Sats(1_361));
    assert_eq!(value(&report, "volume.size.lt_10k"), &Value::Count(1));
    assert!(matches!(
        value(&report, "volume.fiat.CUP.total"),
        Value::Fiat { code, .. } if code == "CUP"
    ));
}

#[tokio::test]
async fn every_cli_dimension_maps_to_an_aggregation_dimension() {
    assert_eq!(dimension(VolumeDimension::Kind), Dimension::Kind);
    assert_eq!(dimension(VolumeDimension::Fiat), Dimension::Fiat);
    assert_eq!(dimension(VolumeDimension::Instance), Dimension::Instance);
    assert_eq!(dimension(VolumeDimension::Period), Dimension::Month);
}

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
            rates: std::collections::BTreeMap::from([("USD".to_string(), usd)]),
        },
    )
    .await
    .expect("rate");
}

#[tokio::test]
async fn converting_prices_each_order_at_the_rate_in_force_when_it_settled() {
    // Arrange: 5k sats at 50k USD/BTC, then 30k sats after the rate moved
    // to 60k. 2.5 + 18 USD.
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    quoted(&pool, ALPHA, FROM, 50_000.0).await;
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    quoted(&pool, ALPHA, FROM + 150, 60_000.0).await;
    settled(&pool, "b", FROM + 200, 30_000, FiatAmount::Fixed(300.0)).await;

    // Act
    let report = report(&pool, &query(), None, Some("USD"), NOW)
        .await
        .expect("report");

    // Assert
    let total = report
        .metrics
        .iter()
        .find(|metric| metric.name == "volume.in.USD.total")
        .expect("the total");
    assert_eq!(total.value, Value::fiat(20.5, "USD"));
    assert!(total.is_inferred());
    assert_eq!(total.error(), Some("rate_age_secs ≤ 100"));
    assert_eq!(value(&report, "volume.in.USD.orders"), &Value::Count(2));
}

#[tokio::test]
async fn the_currency_flag_is_upper_cased_and_checked() {
    assert_eq!(currency("usd").expect("valid"), "USD");
    assert_eq!(currency("EUR").expect("valid"), "EUR");
    assert!(currency("").is_err());
    assert!(currency("us$").is_err());
    assert!(currency("dollars").is_err());
}

/// Over the real corpus the one completed order settled hours before the
/// first rate snapshot: nothing to price it with, and the report says so
/// rather than printing a zero.
#[tokio::test]
async fn an_order_older_than_every_rate_leaves_the_converted_total_missing() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    let now = 1_787_800_000;
    seed_fixtures(&pool, now).await;
    let query = Query {
        range: Range::resolve(Some(1_787_500_000), Some(now), now).expect("window"),
        ..query()
    };

    let report = report(&pool, &query, None, Some("USD"), now)
        .await
        .expect("report");

    assert_eq!(value(&report, "volume.in.USD.total"), &Value::Missing);
    assert_eq!(
        value(&report, "volume.in.USD.unpriced_sats"),
        &Value::Sats(1_361)
    );
    assert_eq!(value(&report, "volume.in.USD.orders"), &Value::Count(0));
}

/// A signed rate event as an instance would publish it.
fn rate_event(keys: &nostr_sdk::prelude::Keys, at: i64, usd: f64) -> nostr_sdk::prelude::Event {
    use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Kind, Tag, Timestamp};

    EventBuilder::new(
        Kind::from_u16(crate::ingest::parse::rates::KIND),
        format!(r#"{{"BTC":{{"USD":{usd}}}}}"#),
    )
    .tags([
        Tag::parse(["d", "mostro-rates"]).expect("tag"),
        Tag::parse(["published_at", &at.to_string()]).expect("tag"),
    ])
    .custom_created_at(Timestamp::from_secs(at as u64))
    .finalize(keys)
    .expect("signing")
}

#[tokio::test]
async fn a_rate_from_an_unvouched_key_cannot_move_the_converted_total() {
    // Arrange: the whole path, not a hand-inserted row — a stranger's
    // snapshot has to pass admission before it can price anything, and in
    // unknown-instance mode nothing about kind 30078 vouches for its
    // signer. Its quote is 1000× the real one, so if it ever reached the
    // book the total could not be mistaken for the honest figure.
    use crate::ingest::pipeline::{Pipeline, Policy};

    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    quoted(&pool, ALPHA, FROM, 50_000.0).await;

    let stranger = nostr_sdk::prelude::Keys::generate();
    let pipeline = Pipeline::new(
        pool.clone(),
        Policy::new(Vec::<String>::new(), true, [Network::Mainnet]),
    );

    // Act
    let outcome = pipeline
        .ingest(
            &rate_event(&stranger, FROM + 50, 50_000_000.0),
            "wss://r",
            NOW,
        )
        .await
        .expect("ingest");

    // Assert: turned away, and the figure is the one the honest snapshot gives.
    assert!(
        matches!(
            outcome,
            crate::ingest::pipeline::IngestOutcome::Rejected(
                crate::ingest::pipeline::Rejection::UnvouchedPublisher { .. }
            )
        ),
        "{outcome:?}"
    );
    let report = report(&pool, &query(), None, Some("USD"), NOW)
        .await
        .expect("report");
    assert_eq!(
        value(&report, "volume.in.USD.total"),
        &Value::fiat(2.5, "USD")
    );
}

/// `--by day` (issue #53). `FROM` is 2026-08-25T23:20:00Z, so everything
/// the seed settles lands on that day and the window holds eight buckets.
#[tokio::test]
async fn a_daily_report_names_each_day_and_keeps_the_quiet_ones() {
    // Arrange
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), FROM)
        .await
        .expect("alpha");

    // Act
    let report = report(&pool, &query(), Some(Dimension::Day), None, NOW)
        .await
        .expect("report");

    // Assert
    assert_eq!(
        value(&report, "volume.2026-08-25.sats"),
        &Value::Sats(5_000)
    );
    assert_eq!(
        value(&report, "volume.2026-08-26.sats"),
        &Value::Sats(0),
        "a day with nothing settled is zero, not absent"
    );
    assert_eq!(
        report
            .metrics
            .iter()
            // One block per day; a currency's own sats row ends the same
            // way and belongs to its block, not to the count of days.
            .filter(|metric| metric.name.ends_with(".sats") && !metric.name.contains(".fiat."))
            .count(),
        8
    );
    assert_eq!(dimension(VolumeDimension::Day), Dimension::Day);
}

#[tokio::test]
async fn a_day_the_archive_predates_reports_no_volume_rather_than_none_traded() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    settled(&pool, "a", FROM + 100, 5_000, FiatAmount::Fixed(50.0)).await;
    let query = Query {
        range: Range::resolve(Some(FROM - 86_400), Some(UNTIL), NOW).expect("window"),
        ..query()
    };

    let report = report(&pool, &query, Some(Dimension::Day), None, NOW)
        .await
        .expect("report");

    assert_eq!(value(&report, "volume.2026-08-24.sats"), &Value::Missing);
    assert_eq!(
        value(&report, "volume.2026-08-25.sats"),
        &Value::Sats(5_000)
    );
}
