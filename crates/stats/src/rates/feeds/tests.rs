//! A hand-built set of feeds and hand-computed expected values
//! (`docs/SPEC.md` §12) for the exchange-rate figures of §6.8.

use super::*;

const NOW: i64 = 100_000;

fn feed(instance: &str, published_at: i64, usd: f64) -> Feed {
    Feed {
        instance: instance.to_string(),
        published_at: Some(published_at),
        rates: BTreeMap::from([("USD".to_string(), usd), ("ARS".to_string(), usd * 1_000.0)]),
    }
}

/// At `now = 100 000`:
///
/// - alpha published 60 s ago (50 000 USD) → **fresh**
/// - beta published 200 s ago (52 000 USD) → **fresh**, 140 s before alpha
/// - gamma published 2 h ago (10 000 USD) → **dead**
/// - delta published 20 min ago (51 000 USD) → **stale**, not dead
/// - epsilon never published → **silent**
///
/// Comparable at an instant: the newest quote is alpha's, and beta's is
/// within five minutes of it; delta's and gamma's are not. Disparity over
/// {50 000, 52 000} → `52 000 / 50 000 − 1` = **0.04**.
fn feeds() -> Vec<Feed> {
    vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Beta", NOW - 200, 52_000.0),
        feed("Gamma", NOW - 7_200, 10_000.0),
        feed("Delta", NOW - 1_200, 51_000.0),
        Feed {
            instance: "Epsilon".to_string(),
            published_at: None,
            rates: BTreeMap::new(),
        },
    ]
}

#[test]
fn a_feed_is_as_old_as_its_latest_snapshot() {
    // Arrange / Act
    let feeds = feeds();

    // Assert
    assert_eq!(feeds[0].age(NOW), Some(60));
    assert_eq!(feeds[4].age(NOW), None, "never published");
}

#[test]
fn freshness_follows_the_valuation_bound_and_the_events_own_expiry() {
    let at = |seconds_ago: i64| feed("x", NOW - seconds_ago, 1.0).freshness(NOW);

    assert_eq!(at(0), Freshness::Fresh);
    assert_eq!(at(MAX_AGE_SECS), Freshness::Fresh, "the bound still prices");
    assert_eq!(at(MAX_AGE_SECS + 1), Freshness::Stale);
    assert_eq!(at(DEAD_AFTER_SECS), Freshness::Stale);
    assert_eq!(at(DEAD_AFTER_SECS + 1), Freshness::Dead);
    assert_eq!(feeds()[4].freshness(NOW), Freshness::Silent);
}

#[test]
fn a_feed_published_in_the_future_is_not_aged_backwards() {
    let ahead = feed("clock", NOW + 500, 50_000.0);

    assert_eq!(ahead.age(NOW), None, "no negative age");
    assert_eq!(ahead.freshness(NOW), Freshness::Silent);
}

#[test]
fn the_summary_counts_the_feeds_by_freshness() {
    let summary = summarise(&feeds(), NOW);

    assert_eq!(summary.feeds, 4, "epsilon has published nothing");
    assert_eq!(summary.fresh, 2);
    assert_eq!(summary.dead, 1);
    assert_eq!(summary.silent, 1);
    assert_eq!(summary.currencies, 2, "USD and ARS");
}

#[test]
fn the_disparity_compares_the_quotes_that_stand_at_the_same_instant() {
    let disparity = disparity(&feeds(), "USD").expect("two comparable quotes");

    assert_eq!(disparity.quoted_by, 4);
    assert_eq!(
        disparity.comparable, 2,
        "delta and gamma are of another hour"
    );
    assert_eq!(disparity.low, 50_000.0);
    assert_eq!(disparity.high, 52_000.0);
    assert!((disparity.ratio - 0.04).abs() < 1e-12);
}

#[test]
fn one_comparable_quote_is_no_disparity() {
    // Only alpha is within five minutes of the newest quote.
    let lonely = vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Gamma", NOW - 7_200, 10_000.0),
    ];

    let disparity = disparity(&lonely, "USD").expect("a quote");

    assert_eq!(disparity.comparable, 1);
    assert_eq!(disparity.ratio, 0.0, "a lone quote disagrees with nobody");
}

#[test]
fn a_currency_nobody_quotes_has_no_disparity() {
    assert!(disparity(&feeds(), "XYZ").is_none());
}

#[test]
fn the_global_report_names_the_summary_then_every_feed() {
    let metrics = report(&feeds(), None, NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        &names[..5],
        [
            "rates.feeds",
            "rates.fresh",
            "rates.dead",
            "rates.silent",
            "rates.currencies",
        ]
    );
    assert_eq!(
        &names[5..11],
        [
            "rates.Alpha.age",
            "rates.Alpha.status",
            "rates.Alpha.currencies",
            "rates.Beta.age",
            "rates.Beta.status",
            "rates.Beta.currencies",
        ]
    );
    assert_eq!(names.len(), 5 + 5 * 3);
    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn a_silent_feed_has_no_age_and_says_so() {
    let metrics = report(&feeds(), None, NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("rates.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("Epsilon.age"), &Value::Missing);
    assert_eq!(value("Epsilon.status"), &Value::Text("silent".into()));
    assert_eq!(value("Epsilon.currencies"), &Value::Count(0));
    assert_eq!(value("Alpha.age"), &Value::Seconds(60));
    assert_eq!(value("Alpha.status"), &Value::Text("fresh".into()));
    assert_eq!(value("Gamma.status"), &Value::Text("dead".into()));
    assert_eq!(value("Delta.status"), &Value::Text("stale".into()));
}

#[test]
fn asking_for_one_currency_adds_its_block_and_the_rate_of_every_instance() {
    let metrics = report(&feeds(), Some("USD"), NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        &names[5..13],
        [
            "rates.USD.quoted_by",
            "rates.USD.comparable",
            "rates.USD.low",
            "rates.USD.high",
            "rates.USD.disparity",
            "rates.USD.Alpha",
            "rates.USD.Beta",
            "rates.USD.Delta",
        ]
    );
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("rates.{name}"))
            .expect("present")
            .value
    };
    assert_eq!(value("USD.Alpha"), &Value::fiat(50_000.0, "USD"));
    assert_eq!(value("USD.low"), &Value::fiat(50_000.0, "USD"));
    assert!(matches!(value("USD.disparity"), Value::Ratio(_)));
    assert!(
        !names.contains(&"rates.USD.Epsilon"),
        "a silent instance quotes nothing"
    );
}

#[test]
fn a_currency_nobody_quotes_reports_the_absence_not_a_zero() {
    let metrics = report(&feeds(), Some("XYZ"), NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("rates.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("XYZ.quoted_by"), &Value::Count(0));
    assert_eq!(value("XYZ.disparity"), &Value::Missing);
    assert_eq!(value("XYZ.low"), &Value::Missing);
}

#[test]
fn no_feeds_at_all_is_an_empty_summary() {
    let metrics = report(&[], None, NOW);

    assert_eq!(metrics.len(), 5);
    assert_eq!(metrics[0].value, Value::Count(0));
    assert_eq!(summarise(&[], NOW).currencies, 0);
}

#[test]
fn a_currency_with_one_comparable_quote_reports_no_disparity_rather_than_zero() {
    let lonely = vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Gamma", NOW - 7_200, 10_000.0),
    ];

    let metrics = report(&lonely, Some("USD"), NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("rates.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("USD.comparable"), &Value::Count(1));
    assert_eq!(value("USD.disparity"), &Value::Missing);
    assert_eq!(value("USD.low"), &Value::fiat(50_000.0, "USD"));
}
