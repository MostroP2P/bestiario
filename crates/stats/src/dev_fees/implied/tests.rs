//! Hand-computed inverses and error bounds (`docs/SPEC.md` §5, §6.6).
//!
//! The window is `[1000, 2000)`. Every instance charges `fee = 0.01` and
//! the assumed dev fee share is `0.25`, so `fee × pct = 0.0025`: a fee of
//! `n` sats implies `n × 400` sats of volume, and one sat of rounding on
//! the fee is `400` sats on the volume.

use super::*;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};

fn fee(id: &str, amount_sats: i64, settled_amount_sats: Option<i64>) -> Fee {
    Fee {
        event_id: id.to_string(),
        order_id: format!("order-{id}"),
        pubkey: "alpha".to_string(),
        instance: "Alpha (alpha)".to_string(),
        created_at: 1_500,
        amount_sats,
        is_duplicate: false,
        order_known: settled_amount_sats.is_some(),
        settled_at: settled_amount_sats.map(|_| 1_400),
        fee_in_force: Some(0.01),
        settled_amount_sats,
    }
}

fn data(fees: Vec<Fee>) -> DevFeeData {
    DevFeeData {
        fees,
        settlements: vec![],
    }
}

fn quarter(_: &str) -> f64 {
    0.25
}

/// f1 (100 sats, order of 40k), f2 (200 sats, order of 100k), f3 (300
/// sats, orphan): implied 40k + 80k + 120k = **240k ± 1200**. f4 has no
/// fee in force and f5 a zero one: **2 not invertible**. f6 is a
/// duplicate and f7 is outside the window: ignored.
fn dataset() -> DevFeeData {
    data(vec![
        fee("f1", 100, Some(40_000)),
        fee("f2", 200, Some(100_000)),
        fee("f3", 300, None),
        Fee {
            fee_in_force: None,
            ..fee("f4", 50, Some(1_000))
        },
        Fee {
            fee_in_force: Some(0.0),
            ..fee("f5", 10, Some(1_000))
        },
        Fee {
            is_duplicate: true,
            ..fee("f6", 999, Some(1_000))
        },
        Fee {
            created_at: 2_500,
            ..fee("f7", 999, Some(1_000))
        },
    ])
}

#[test]
fn the_implied_volume_is_each_fee_divided_by_fee_times_pct() {
    // Arrange / Act
    let implied = summarise(&dataset(), WINDOW, &quarter);

    // Assert
    assert_eq!(implied.volume_sats, Some(240_000));
    assert_eq!(implied.inverted, 3);
}

#[test]
fn the_error_is_one_sat_per_fee_amplified_by_the_inverse() {
    let implied = summarise(&dataset(), WINDOW, &quarter);

    assert_eq!(implied.error_sats, 1_200);
}

#[test]
fn a_missing_rate_and_an_instance_charging_nothing_are_counted_apart() {
    // Arrange / Act
    let implied = summarise(&dataset(), WINDOW, &quarter);

    // Assert: f4 has no fee in force, f5 charges zero — not the same gap.
    assert_eq!(implied.no_fee_in_force, 1);
    assert_eq!(implied.zero_fee, 1);
    assert_eq!(implied.not_invertible(), 2);
}

#[test]
fn the_comparison_is_over_the_fees_whose_order_settled() {
    let implied = summarise(&dataset(), WINDOW, &quarter);

    // f1 and f2: implied 120k against observed 140k. f4 and f5 name settled
    // orders too, so the observed side has all four; the ratio has two.
    assert_eq!(implied.matched, 2);
    assert_eq!(implied.matched_implied_sats, 120_000);
    assert_eq!(implied.matched_observed_sats, 140_000);
    assert_eq!(implied.with_fee_orders, 4);
    assert_eq!(implied.with_fee_volume_sats, Some(142_000));
    let ratio = implied.implied_vs_observed().expect("a ratio");
    assert!((ratio - (120_000.0 / 140_000.0 - 1.0)).abs() < 1e-12);
}

#[test]
fn the_ratio_carries_the_rounding_bound_of_the_fees_it_is_over() {
    // Arrange / Act: f1 and f2 round to ±400 sats each on a 140k
    // denominator.
    let implied = summarise(&dataset(), WINDOW, &quarter);

    // Assert
    assert_eq!(implied.matched_error_sats, 800);
    let error = implied.implied_vs_observed_error().expect("an error");
    assert!((error - 800.0 / 140_000.0).abs() < 1e-12);
}

#[test]
fn an_order_that_never_settled_is_in_neither_side_of_the_comparison() {
    // Arrange: the loader leaves `settled_amount_sats` empty for an order
    // that was canceled after the fee was paid (SPEC §6.6 counts `success`).
    let canceled = Fee {
        order_known: true,
        settled_at: None,
        settled_amount_sats: None,
        ..fee("canceled", 100, None)
    };

    // Act
    let implied = summarise(&data(vec![canceled]), WINDOW, &quarter);

    // Assert: inverted, and out of both observed figures.
    assert_eq!(implied.inverted, 1);
    assert_eq!(implied.volume_sats, Some(40_000));
    assert_eq!(implied.with_fee_orders, 0);
    assert_eq!(implied.with_fee_volume_sats, None);
    assert_eq!(implied.matched, 0);
    assert_eq!(implied.implied_vs_observed(), None);
}

#[test]
fn a_settled_order_still_at_amt_zero_is_left_out_of_the_ratio() {
    // Arrange: a market-price order the projection never saw amended
    // (SPEC §3: `amt` is 0 until taken) would divide by zero volume.
    let fees = vec![
        fee("priced", 100, Some(40_000)),
        fee("market", 100, Some(0)),
    ];

    // Act
    let implied = summarise(&data(fees), WINDOW, &quarter);

    // Assert: both in the observed sum, only the priced one in the ratio.
    assert_eq!(implied.with_fee_orders, 2);
    assert_eq!(implied.with_fee_volume_sats, Some(40_000));
    assert_eq!(implied.matched, 1);
    assert_eq!(implied.zero_amount_orders, 1);
    assert_eq!(implied.implied_vs_observed(), Some(0.0));
}

#[test]
fn nothing_but_zero_amount_orders_is_a_missing_ratio_that_says_why() {
    let implied = summarise(&data(vec![fee("market", 100, Some(0))]), WINDOW, &quarter);
    let metrics = metrics("dev_fees", &implied);

    assert_eq!(implied.implied_vs_observed(), None);
    assert_eq!(metrics[2].value, Value::Missing);
    let error = metrics[2].error().expect("error");
    assert!(error.contains("amt = 0"), "{error}");
    assert!(!error.contains("implied / observed"), "{error}");
}

#[test]
fn the_assumed_share_is_looked_up_per_instance() {
    let data = data(vec![
        fee("a", 100, None),
        Fee {
            pubkey: "beta".to_string(),
            ..fee("b", 100, None)
        },
    ]);
    let pct = |pubkey: &str| if pubkey == "beta" { 0.5 } else { 0.25 };

    let implied = summarise(&data, WINDOW, &pct);

    // 100 / 0.0025 + 100 / 0.005.
    assert_eq!(implied.volume_sats, Some(60_000));
    assert_eq!(implied.assumed_pcts, vec![0.25, 0.5]);
}

#[test]
fn no_fees_is_zero_volume_and_fees_that_cannot_be_inverted_is_none() {
    let none = summarise(&DevFeeData::default(), WINDOW, &quarter);
    assert_eq!(none.volume_sats, Some(0));
    assert_eq!(none.error_sats, 0);
    assert_eq!(none.with_fee_volume_sats, None);

    let data = data(vec![Fee {
        fee_in_force: None,
        ..fee("f", 100, None)
    }]);
    let stuck = summarise(&data, WINDOW, &quarter);
    assert_eq!(stuck.volume_sats, None);
    assert_eq!(stuck.not_invertible(), 1);
}

#[test]
fn the_metrics_are_the_inferred_volume_beside_the_observed_one_and_their_ratio() {
    let metrics = metrics("dev_fees", &summarise(&dataset(), WINDOW, &quarter));

    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "dev_fees.implied_volume",
            "dev_fees.with_fee_volume",
            "dev_fees.implied_vs_observed",
        ]
    );
    assert!(metrics[0].is_inferred());
    assert_eq!(metrics[0].value, Value::Sats(240_000));
    assert!(!metrics[1].is_inferred(), "the observed side is observed");
    assert_eq!(metrics[1].value, Value::Sats(142_000));
    assert!(metrics[2].is_inferred());
    assert!(matches!(metrics[2].value, Value::Ratio(_)));
}

#[test]
fn the_observed_side_does_not_move_with_what_could_be_inverted() {
    // Arrange: the same settled order of 40k, once with a rate to divide
    // by and once without.
    let with_rate = data(vec![fee("f", 100, Some(40_000))]);
    let without = data(vec![Fee {
        fee_in_force: None,
        ..fee("f", 100, Some(40_000))
    }]);

    // Act
    let priced = metrics("dev_fees", &summarise(&with_rate, WINDOW, &quarter));
    let unpriced = metrics("dev_fees", &summarise(&without, WINDOW, &quarter));

    // Assert: an observed figure is observed either way.
    assert_eq!(priced[1].value, Value::Sats(40_000));
    assert_eq!(unpriced[1].value, Value::Sats(40_000));
    assert_eq!(unpriced[0].value, Value::Missing);
}

#[test]
fn a_volume_with_fees_left_out_is_reported_as_a_lower_bound() {
    let metrics = metrics("dev_fees", &summarise(&dataset(), WINDOW, &quarter));
    let error = metrics[0].error().expect("error");

    assert!(
        error.contains("lower bound: 3 of 5 fees inverted"),
        "{error}"
    );
    assert!(error.contains("1 fee with no fee in force"), "{error}");
    assert!(error.contains("1 fee charging a zero fee"), "{error}");
}

#[test]
fn the_error_column_states_the_bound_the_assumption_and_what_was_left_out() {
    let metrics = metrics("dev_fees", &summarise(&dataset(), WINDOW, &quarter));
    let error = metrics[0].error().expect("error");

    assert!(error.contains("±1200 sats"), "{error}");
    assert!(error.contains("worst case"), "{error}");
    assert!(error.contains("pct assumed 0.25"), "{error}");
}

#[test]
fn the_ratio_column_names_its_fees_in_the_singular_when_there_is_one() {
    let implied = summarise(&data(vec![fee("f1", 100, Some(40_000))]), WINDOW, &quarter);
    let rows = metrics("dev_fees", &implied);
    let error = rows[2].error().expect("error");

    assert!(error.contains("over the 1 fee whose order"), "{error}");
    assert!(error.contains("±0.0100 from the rounding"), "{error}");
}

#[test]
fn many_assumed_shares_are_given_as_a_range_instead_of_a_list() {
    // Arrange: one instance per share, more than the error column lists.
    let fees: Vec<Fee> = ["a", "b", "c", "d"]
        .iter()
        .map(|pubkey| Fee {
            pubkey: (*pubkey).to_string(),
            ..fee(pubkey, 100, None)
        })
        .collect();
    let pct = |pubkey: &str| match pubkey {
        "a" => 0.20,
        "b" => 0.30,
        "c" => 0.40,
        _ => 0.60,
    };

    // Act
    let error = metrics("dev_fees", &summarise(&data(fees), WINDOW, &pct))[0]
        .error()
        .expect("error")
        .to_string();

    // Assert
    assert!(
        error.contains("pct assumed 0.20–0.60 across 4 instances"),
        "{error}"
    );
}

#[test]
fn a_ratio_over_no_matched_order_is_missing() {
    let data = data(vec![fee("orphan", 100, None)]);

    let metrics = metrics("dev_fees", &summarise(&data, WINDOW, &quarter));

    assert_eq!(metrics[2].value, Value::Missing);
    assert_eq!(metrics[1].value, Value::Missing);
}

#[test]
fn a_market_price_order_left_out_of_the_comparison_is_named_beside_it() {
    // One fee whose order settled at a real amount, so the ratio exists,
    // and one whose order is known at `amt = 0` — a market-price order
    // nobody amended, which has no observed volume to compare against.
    let data = data(vec![
        fee("real", 100, Some(40_000)),
        fee("market", 200, Some(0)),
    ]);

    let implied = summarise(&data, WINDOW, &quarter);
    let metrics = metrics("dev_fees", &implied);
    let comparison = metrics
        .iter()
        .find(|metric| metric.name == "dev_fees.implied_vs_observed")
        .expect("the ratio");

    assert_eq!(implied.zero_amount_orders, 1);
    assert_eq!(implied.matched, 1, "only the one with an amount");
    let error = comparison.error().expect("an error column");
    assert!(
        error.contains("1 fee left out at amt = 0"),
        "the reader is told what the ratio does not cover: {error}"
    );
}
