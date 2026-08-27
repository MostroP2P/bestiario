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

fn silent(instance: &str) -> Feed {
    Feed {
        instance: instance.to_string(),
        published_at: None,
        rates: BTreeMap::new(),
    }
}

/// At `now = 100 000`:
///
/// - alpha published 60 s ago (50 000 USD) → **fresh**
/// - beta published 200 s ago (52 000 USD) → **fresh**
/// - gamma published 2 h ago (10 000 USD) → **dead**
/// - delta published 500 s ago (51 000 USD) → **stale**, not dead
/// - epsilon never published → **silent**
///
/// Fresh at `now`: alpha's and beta's quotes. Disparity over
/// {50 000, 52 000} → `52 000 / 50 000 − 1` = **0.04**.
fn feeds() -> Vec<Feed> {
    vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Beta", NOW - 200, 52_000.0),
        feed("Gamma", NOW - 7_200, 10_000.0),
        feed("Delta", NOW - 500, 51_000.0),
        silent("Epsilon"),
    ]
}

fn value<'a>(metrics: &'a [Metric], name: &str) -> &'a Value {
    &metrics
        .iter()
        .find(|metric| metric.name == format!("rates.{name}"))
        .expect("present")
        .value
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
fn the_expiry_a_kind_30078_event_carries_is_ten_minutes() {
    // The `expiration` tag of both captured snapshots sits 600 s after
    // `published_at`; a feed may not outlive the event that carried it.
    assert_eq!(DEAD_AFTER_SECS, 600);
}

#[test]
fn a_feed_published_in_the_future_is_skewed_rather_than_silent() {
    let ahead = feed("clock", NOW + 500, 50_000.0);

    assert_eq!(ahead.age(NOW), None, "no negative age");
    assert_eq!(
        ahead.freshness(NOW),
        Freshness::Skewed,
        "it published; the clock is what nobody shares"
    );
}

#[test]
fn an_unrepresentable_distance_from_now_is_dead_not_fresh() {
    let ancient = Feed {
        instance: "overflow".to_string(),
        published_at: Some(i64::MIN),
        rates: BTreeMap::from([("USD".to_string(), 1.0)]),
    };

    assert_eq!(ancient.freshness(i64::MAX), Freshness::Dead);
    assert_eq!(ancient.age(i64::MAX), None, "no age it can state");
}

#[test]
fn the_summary_counts_the_feeds_by_freshness() {
    let summary = summarise(&feeds(), NOW);

    assert_eq!(summary.feeds, 4, "epsilon has published nothing");
    assert_eq!(summary.fresh, 2);
    assert_eq!(summary.stale, 1);
    assert_eq!(summary.dead, 1);
    assert_eq!(summary.silent, 1);
    assert_eq!(summary.skewed, 0);
    assert_eq!(summary.currencies, 2, "USD and ARS");
}

#[test]
fn every_bucket_reconciles_with_the_feeds_that_published() {
    let mut feeds = feeds();
    feeds.push(feed("Zeta", NOW + 500, 1.0));
    let summary = summarise(&feeds, NOW);

    assert_eq!(
        summary.fresh + summary.stale + summary.dead + summary.skewed,
        summary.feeds,
        "a publisher is in exactly one published bucket"
    );
    assert_eq!(
        summary.feeds + summary.silent,
        feeds.len() as u64,
        "and every feed is in exactly one bucket"
    );
    assert_eq!(summary.skewed, 1);
}

#[test]
fn the_disparity_compares_the_quotes_that_are_fresh_now() {
    let disparity = disparity(&feeds(), "USD", NOW).expect("two comparable quotes");

    assert_eq!(disparity.quoted_by, 4);
    assert_eq!(
        disparity.comparable, 2,
        "delta is stale and gamma is dead: neither prices anything now"
    );
    assert_eq!(disparity.low, Some(50_000.0));
    assert_eq!(disparity.high, Some(52_000.0));
    assert!((disparity.ratio.expect("a ratio") - 0.04).abs() < 1e-12);
}

#[test]
fn quotes_that_have_all_expired_are_no_disparity_at_all() {
    // Two dead snapshots from two hours ago disagree with each other, not
    // about what anybody quotes now.
    let dead = vec![
        feed("Alpha", NOW - 7_200, 50_000.0),
        feed("Beta", NOW - 7_100, 100_000.0),
    ];

    let disparity = disparity(&dead, "USD", NOW).expect("both quote USD");

    assert_eq!(disparity.quoted_by, 2);
    assert_eq!(disparity.comparable, 0);
    assert_eq!(disparity.low, None);
    assert_eq!(disparity.high, None);
    assert_eq!(disparity.ratio, None);
}

#[test]
fn a_future_quote_neither_sets_the_bounds_nor_hides_a_live_one() {
    // The skewed feed is the newest timestamp of the three, and quotes an
    // absurd rate; the disparity is still the live one's.
    let feeds = vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Beta", NOW - 90, 51_000.0),
        feed("Clock", NOW + 10_000, 1.0),
    ];

    let disparity = disparity(&feeds, "USD", NOW).expect("three quote USD");

    assert_eq!(disparity.quoted_by, 3);
    assert_eq!(disparity.comparable, 2, "the future one prices nothing");
    assert_eq!(disparity.low, Some(50_000.0));
    assert_eq!(disparity.high, Some(51_000.0));
    assert!((disparity.ratio.expect("a ratio") - 0.02).abs() < 1e-12);
}

#[test]
fn one_comparable_quote_is_no_disparity() {
    // Only alpha's quote is fresh now.
    let lonely = vec![
        feed("Alpha", NOW - 60, 50_000.0),
        feed("Gamma", NOW - 7_200, 10_000.0),
    ];

    let disparity = disparity(&lonely, "USD", NOW).expect("a quote");

    assert_eq!(disparity.comparable, 1);
    assert_eq!(disparity.low, Some(50_000.0));
    assert_eq!(disparity.ratio, None, "a lone quote disagrees with nobody");
}

#[test]
fn a_currency_nobody_quotes_has_no_disparity() {
    assert!(disparity(&feeds(), "XYZ", NOW).is_none());
}

#[test]
fn two_quotes_whose_quotient_is_not_a_number_report_no_ratio() {
    // Both rates are finite and positive — the domain `Feed::rate` admits —
    // but `f64::MAX / f64::MIN_POSITIVE` is infinity, and an infinity is
    // not a disparity.
    let extremes = vec![
        feed("Dear", NOW - 60, f64::MAX),
        feed("Cheap", NOW - 61, f64::MIN_POSITIVE),
    ];

    let disparity = disparity(&extremes, "USD", NOW).expect("both quote USD");

    assert_eq!(disparity.comparable, 2);
    assert_eq!(disparity.low, Some(f64::MIN_POSITIVE), "still observed");
    assert_eq!(disparity.high, Some(f64::MAX));
    assert_eq!(disparity.ratio, None);

    let metrics = report(&extremes, Some("USD"), NOW);
    assert_eq!(value(&metrics, "USD.disparity"), &Value::Missing);
}

#[test]
fn a_finite_disparity_between_far_apart_quotes_is_still_reported() {
    // The guard rejects only what is not a number: a ratio of a million
    // is a disparity, however implausible, and stays one.
    let far = vec![
        feed("Dear", NOW - 60, 1_000_000.0),
        feed("Cheap", NOW - 61, 1.0),
    ];

    let disparity = disparity(&far, "USD", NOW).expect("both quote USD");

    assert_eq!(disparity.ratio, Some(999_999.0));
}

#[test]
fn the_global_report_names_every_bucket_then_every_feed() {
    let metrics = report(&feeds(), None, NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        &names[..7],
        [
            "rates.feeds",
            "rates.fresh",
            "rates.stale",
            "rates.dead",
            "rates.silent",
            "rates.skewed",
            "rates.currencies",
        ]
    );
    assert_eq!(
        &names[7..13],
        [
            "rates.Alpha.age",
            "rates.Alpha.status",
            "rates.Alpha.currencies",
            "rates.Beta.age",
            "rates.Beta.status",
            "rates.Beta.currencies",
        ]
    );
    assert_eq!(names.len(), 7 + 5 * 3);
    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn the_reported_counts_are_the_statuses_below_them() {
    let mut feeds = feeds();
    feeds.push(feed("Zeta", NOW + 500, 1.0));
    let metrics = report(&feeds, None, NOW);
    let statuses = |wanted: &str| {
        metrics
            .iter()
            .filter(|metric| metric.name.ends_with(".status"))
            .filter(|metric| metric.value == Value::Text(wanted.to_string()))
            .count() as i64
    };

    for bucket in ["fresh", "stale", "dead", "silent", "skewed"] {
        assert_eq!(
            value(&metrics, bucket),
            &Value::Count(statuses(bucket)),
            "the `{bucket}` count is the feeds the report calls {bucket}"
        );
    }
}

#[test]
fn a_silent_feed_has_no_age_and_says_so() {
    let metrics = report(&feeds(), None, NOW);

    assert_eq!(value(&metrics, "Epsilon.age"), &Value::Missing);
    assert_eq!(
        value(&metrics, "Epsilon.status"),
        &Value::Text("silent".into())
    );
    assert_eq!(value(&metrics, "Epsilon.currencies"), &Value::Count(0));
    assert_eq!(value(&metrics, "Alpha.age"), &Value::Seconds(60));
    assert_eq!(
        value(&metrics, "Alpha.status"),
        &Value::Text("fresh".into())
    );
    assert_eq!(value(&metrics, "Gamma.status"), &Value::Text("dead".into()));
    assert_eq!(
        value(&metrics, "Delta.status"),
        &Value::Text("stale".into())
    );
}

#[test]
fn a_skewed_feed_is_named_as_one_and_still_counted_as_a_publisher() {
    let metrics = report(&[feed("Clock", NOW + 500, 50_000.0)], None, NOW);

    assert_eq!(value(&metrics, "feeds"), &Value::Count(1));
    assert_eq!(value(&metrics, "silent"), &Value::Count(0));
    assert_eq!(value(&metrics, "skewed"), &Value::Count(1));
    assert_eq!(value(&metrics, "Clock.age"), &Value::Missing);
    assert_eq!(
        value(&metrics, "Clock.status"),
        &Value::Text("skewed".into())
    );
}

#[test]
fn asking_for_one_currency_adds_its_block_and_the_rate_of_every_instance() {
    let metrics = report(&feeds(), Some("USD"), NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        &names[7..15],
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
    assert_eq!(value(&metrics, "USD.Alpha"), &Value::fiat(50_000.0, "USD"));
    assert_eq!(value(&metrics, "USD.low"), &Value::fiat(50_000.0, "USD"));
    assert!(matches!(value(&metrics, "USD.disparity"), Value::Ratio(_)));
    assert!(
        !names.contains(&"rates.USD.Epsilon"),
        "a silent instance quotes nothing"
    );
}

#[test]
fn a_currency_only_dead_feeds_quote_reports_the_absence_not_a_ratio() {
    let dead = vec![
        feed("Alpha", NOW - 7_200, 50_000.0),
        feed("Beta", NOW - 7_100, 100_000.0),
    ];

    let metrics = report(&dead, Some("USD"), NOW);

    assert_eq!(value(&metrics, "USD.quoted_by"), &Value::Count(2));
    assert_eq!(value(&metrics, "USD.comparable"), &Value::Count(0));
    assert_eq!(value(&metrics, "USD.low"), &Value::Missing);
    assert_eq!(value(&metrics, "USD.high"), &Value::Missing);
    assert_eq!(value(&metrics, "USD.disparity"), &Value::Missing);
}

#[test]
fn a_currency_nobody_quotes_reports_the_absence_not_a_zero() {
    let metrics = report(&feeds(), Some("XYZ"), NOW);

    assert_eq!(value(&metrics, "XYZ.quoted_by"), &Value::Count(0));
    assert_eq!(value(&metrics, "XYZ.disparity"), &Value::Missing);
    assert_eq!(value(&metrics, "XYZ.low"), &Value::Missing);
}

#[test]
fn a_rate_that_is_not_a_price_is_not_quoted() {
    let broken = Feed {
        instance: "Broken".to_string(),
        published_at: Some(NOW - 60),
        rates: BTreeMap::from([("USD".to_string(), f64::NAN), ("EUR".to_string(), -1.0)]),
    };

    assert_eq!(broken.rate("USD"), None);
    assert_eq!(broken.rate("EUR"), None);
    assert!(disparity(&[broken], "USD", NOW).is_none());
}

#[test]
fn no_feeds_at_all_is_an_empty_summary() {
    let metrics = report(&[], None, NOW);

    assert_eq!(metrics.len(), 7);
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

    assert_eq!(value(&metrics, "USD.comparable"), &Value::Count(1));
    assert_eq!(value(&metrics, "USD.disparity"), &Value::Missing);
    assert_eq!(value(&metrics, "USD.low"), &Value::fiat(50_000.0, "USD"));
}
