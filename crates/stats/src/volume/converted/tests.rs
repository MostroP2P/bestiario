//! Hand-computed conversions over the dataset of the parent tests
//! (`docs/SPEC.md` §12): every expected figure below is worked out from the
//! order sizes and the rate book by hand, not by the code under test.

use std::collections::BTreeMap;

use super::*;
use crate::activity::{Direction, Status};
use crate::rates::Snapshot;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};

fn order(id: &str, pubkey: &str, success_at: i64, sats: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: pubkey.to_string(),
        instance: format!("{pubkey} ({pubkey})"),
        created_at: 500,
        status: Status::Success,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec![],
        amount_sats: sats,
        fiat_amount: Some(sats as f64 / 100.0),
        taken_at: None,
        success_at: Some(success_at),
        canceled_at: None,
        expires_at: None,
    }
}

fn snapshot(pubkey: &str, published_at: i64, usd: f64) -> Snapshot {
    Snapshot {
        pubkey: pubkey.to_string(),
        published_at,
        rates: BTreeMap::from([("USD".to_string(), usd)]),
    }
}

/// alpha publishes at 1000 (50k), 1800 (50k again) and 2000 (52k); beta
/// at 1500 (51k).
fn book() -> RateBook {
    RateBook::new(vec![
        snapshot("alpha", 1_000, 50_000.0),
        snapshot("beta", 1_500, 51_000.0),
        snapshot("alpha", 1_800, 50_000.0),
        snapshot("alpha", 2_000, 52_000.0),
    ])
}

/// Four of alpha's orders, all priced at 50k USD/BTC — the first three on
/// its 1000 snapshot, the last on its 1800 one: 5k, 30k, 150k and 2M sats
/// are 2.5, 15, 75 and 1000 USD.
fn priced() -> Vec<Order> {
    vec![
        order("a", "alpha", 1_100, 5_000),
        order("b", "alpha", 1_200, 30_000),
        order("c", "alpha", 1_300, 150_000),
        order("d", "alpha", 1_900, 2_000_000),
    ]
}

#[test]
fn the_total_is_each_order_at_the_rate_its_instance_knew_when_it_settled() {
    // Arrange / Act
    let converted = convert(&priced(), WINDOW, &book(), "USD");

    // Assert
    assert_eq!(converted.total, Some(2.5 + 15.0 + 75.0 + 1_000.0));
    assert_eq!(converted.priced, 4);
    assert_eq!(converted.unpriced, 0);
    assert_eq!(converted.unpriced_sats, Some(0));
    assert_eq!(converted.fallbacks(), 0);
}

#[test]
fn the_rate_age_reported_is_the_oldest_quote_used() {
    let converted = convert(&priced(), WINDOW, &book(), "USD");

    // `c` settled at 1300 on the snapshot from 1000, the oldest quote used;
    // `d` at 1900 is on the 1800 one.
    assert_eq!(converted.rate_age_max_secs, Some(300));
}

#[test]
fn only_the_orders_completed_in_the_window_are_converted() {
    let mut orders = priced();
    orders.push(order("late", "alpha", 2_500, 1_000_000));
    orders.push(Order {
        status: Status::Pending,
        success_at: None,
        ..order("open", "alpha", 0, 1_000_000)
    });

    let converted = convert(&orders, WINDOW, &book(), "USD");

    assert_eq!(converted.priced, 4);
    assert_eq!(converted.total, Some(1_092.5));
}

#[test]
fn an_order_priced_on_another_instance_s_snapshot_is_counted_as_a_fallback() {
    // gamma published nothing; at 1600 the newest snapshot is beta's.
    let orders = vec![order("g", "gamma", 1_600, 100_000)];

    let converted = convert(&orders, WINDOW, &book(), "USD");

    assert_eq!(converted.total, Some(51.0));
    assert_eq!(converted.fallbacks(), 1);
    assert_eq!(converted.rate_age_max_secs, Some(100));
}

#[test]
fn an_order_settled_before_any_rate_is_excluded_and_its_sats_reported() {
    let orders = vec![
        order("a", "alpha", 1_100, 5_000),
        order("early", "alpha", 1_050, 7_000),
    ];
    let book = RateBook::new(vec![snapshot("alpha", 1_080, 50_000.0)]);

    let converted = convert(&orders, WINDOW, &book, "USD");

    assert_eq!(converted.total, Some(2.5));
    assert_eq!(converted.priced, 1);
    assert_eq!(converted.unpriced, 1);
    assert_eq!(converted.unpriced_sats, Some(7_000));
}

#[test]
fn a_currency_nobody_quotes_leaves_everything_unpriced() {
    let converted = convert(&priced(), WINDOW, &book(), "EUR");

    assert_eq!(converted.priced, 0);
    assert_eq!(converted.unpriced, 4);
    assert_eq!(converted.unpriced_sats, Some(2_185_000));
    assert_eq!(converted.rate_age_max_secs, None);
}

#[test]
fn every_converted_metric_is_inferred_and_carries_the_rate_age() {
    let metrics = metrics("volume", &convert(&priced(), WINDOW, &book(), "USD"));

    assert!(!metrics.is_empty());
    for metric in &metrics {
        assert!(metric.is_inferred(), "{} is not inferred", metric.name);
        assert!(
            metric.error().is_some_and(|error| !error.is_empty()),
            "{} has no error",
            metric.name
        );
    }
    let total = metrics
        .iter()
        .find(|metric| metric.name == "volume.in.USD.total")
        .expect("the total");
    assert_eq!(total.value, Value::fiat(1_092.5, "USD"));
    assert!(
        total.error().expect("error").contains("300"),
        "the rate age is in the error: {:?}",
        total.error()
    );
}

#[test]
fn the_metrics_name_the_priced_count_the_unpriced_sats_and_the_age() {
    let metrics = metrics("volume", &convert(&priced(), WINDOW, &book(), "USD"));
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "volume.in.USD.total",
            "volume.in.USD.orders",
            "volume.in.USD.unpriced_sats",
            "volume.in.USD.rate_age_max",
        ]
    );
    assert_eq!(metrics[1].value, Value::Count(4));
    assert_eq!(metrics[2].value, Value::Sats(0));
    assert_eq!(metrics[3].value, Value::Seconds(300));
}

#[test]
fn fallbacks_and_unpriced_orders_are_named_in_the_error_of_the_total() {
    let orders = vec![
        order("g", "gamma", 1_600, 100_000),
        order("early", "alpha", 1_001, 7_000),
    ];
    let book = RateBook::new(vec![snapshot("beta", 1_500, 51_000.0)]);

    let metrics = metrics("volume", &convert(&orders, WINDOW, &book, "USD"));
    let error = metrics[0].error().expect("error");

    assert!(error.contains("1 at another instance's rate"), "{error}");
    assert!(error.contains("1 with no usable rate"), "{error}");
}

#[test]
fn nothing_completed_is_a_zero_total_with_no_age() {
    let metrics = metrics("volume", &convert(&[], WINDOW, &book(), "USD"));

    assert_eq!(metrics[0].value, Value::fiat(0.0, "USD"));
    assert_eq!(metrics[3].value, Value::Missing);
}

#[test]
fn everything_unpriced_is_a_missing_total_not_a_zero() {
    let metrics = metrics("volume", &convert(&priced(), WINDOW, &book(), "EUR"));

    assert_eq!(metrics[0].value, Value::Missing);
    assert_eq!(metrics[2].value, Value::Sats(2_185_000));
}

/// A price so large that a big order overflows the conversion: `f64::MAX`
/// times anything above one is infinite.
#[test]
fn an_order_whose_conversion_is_not_finite_is_a_numeric_failure_not_a_price() {
    // Arrange
    let book = RateBook::new(vec![snapshot("alpha", 1_000, f64::MAX)]);
    let orders = vec![order("huge", "alpha", 1_100, i64::MAX)];

    // Act
    let converted = convert(&orders, WINDOW, &book, "USD");

    // Assert: it is not priced, and it is not silently absent either.
    assert_eq!(converted.priced, 0);
    assert_eq!(converted.unusable, 1);
    assert_eq!(converted.unusable_sats, Some(i64::MAX));
    assert_eq!(converted.total, Some(0.0), "nothing was added to the sum");
}

#[test]
fn a_numeric_failure_is_named_in_the_qualification() {
    let book = RateBook::new(vec![snapshot("alpha", 1_000, f64::MAX)]);
    let orders = vec![order("huge", "alpha", 1_100, i64::MAX)];

    let metrics = metrics("volume", &convert(&orders, WINDOW, &book, "USD"));
    let total = &metrics[0];

    assert_eq!(total.value, Value::Missing);
    assert!(
        total.error().unwrap_or_default().contains("1 unusable"),
        "{:?}",
        total.error()
    );
}

#[test]
fn a_total_that_stops_being_finite_while_summing_is_reported_as_missing() {
    // Two orders each convertible on their own, whose sum is not.
    let book = RateBook::new(vec![snapshot("alpha", 1_000, 1e308)]);
    let orders = vec![
        order("a", "alpha", 1_100, 100_000_000),
        order("b", "alpha", 1_200, 100_000_000),
    ];

    let converted = convert(&orders, WINDOW, &book, "USD");

    assert_eq!(converted.priced, 2);
    assert_eq!(converted.total_of_priced(), None, "the sum left f64");
    assert_eq!(metrics("volume", &converted)[0].value, Value::Missing);
}

#[test]
fn excluded_sats_beyond_i64_are_reported_as_missing_rather_than_wrapped() {
    // No rate at all, so both orders are excluded and their sats add up to
    // more than i64 can hold.
    let book = RateBook::new(vec![]);
    let orders = vec![
        order("a", "alpha", 1_100, i64::MAX - 1),
        order("b", "alpha", 1_200, 2),
    ];

    let converted = convert(&orders, WINDOW, &book, "USD");

    assert_eq!(converted.unpriced, 2);
    assert_eq!(converted.unpriced_sats, None);
    let unpriced = &metrics("volume", &converted)[2];
    assert_eq!(unpriced.value, Value::Missing);
}

#[test]
fn every_instance_whose_rate_was_borrowed_is_named() {
    // Arrange: two orders of an instance that never published, priced from
    // two different publishers. A bare count would hide that the figure
    // mixes sources.
    let book = RateBook::new(vec![
        snapshot("aaaaaaaa1111", 1_000, 50_000.0),
        snapshot("bbbbbbbb2222", 1_500, 60_000.0),
    ]);
    let orders = vec![
        order("a", "silent", 1_100, 100_000_000),
        order("b", "silent", 1_600, 100_000_000),
    ];

    // Act
    let converted = convert(&orders, WINDOW, &book, "USD");
    let qualification = metrics("volume", &converted)[0]
        .error()
        .unwrap_or_default()
        .to_string();

    // Assert
    assert_eq!(converted.fallbacks(), 2);
    assert_eq!(
        converted.fallback_sources(),
        vec![
            ("aaaaaaaa1111".to_string(), 1, 100_000_000),
            ("bbbbbbbb2222".to_string(), 1, 100_000_000),
        ]
    );
    assert!(qualification.contains("aaaaaaaa"), "{qualification}");
    assert!(qualification.contains("bbbbbbbb"), "{qualification}");
}

#[test]
fn an_order_whose_only_quotes_are_stale_is_not_described_as_never_quoted() {
    // A snapshot exists for the currency, just too old to price with. The
    // reader has to be able to tell a dead feed from an unquoted currency.
    let book = RateBook::new(vec![snapshot("alpha", 1_000, 50_000.0)]);
    let orders = vec![order("late", "alpha", 1_900, 5_000)];

    let converted = convert(&orders, WINDOW, &book, "USD");
    let qualification = metrics("volume", &converted)[0]
        .error()
        .unwrap_or_default()
        .to_string();

    assert_eq!(converted.unpriced, 1);
    assert!(
        !qualification.contains("no instance had a rate"),
        "{qualification}"
    );
    assert!(qualification.contains("no usable rate"), "{qualification}");
}
