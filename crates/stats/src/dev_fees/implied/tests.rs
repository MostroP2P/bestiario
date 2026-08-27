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

fn fee(id: &str, amount_sats: i64, order_amount_sats: Option<i64>) -> Fee {
    Fee {
        event_id: id.to_string(),
        order_id: format!("order-{id}"),
        pubkey: "alpha".to_string(),
        instance: "Alpha (alpha)".to_string(),
        created_at: 1_500,
        amount_sats,
        is_duplicate: false,
        order_known: order_amount_sats.is_some(),
        settled_at: order_amount_sats.map(|_| 1_400),
        fee_in_force: Some(0.01),
        order_amount_sats,
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
    DevFeeData {
        fees: vec![
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
        ],
        settlements: vec![],
    }
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
fn fees_with_no_fee_in_force_or_a_zero_one_cannot_be_inverted() {
    let implied = summarise(&dataset(), WINDOW, &quarter);

    assert_eq!(implied.not_invertible, 2);
}

#[test]
fn the_comparison_is_over_the_fees_whose_order_is_known() {
    let implied = summarise(&dataset(), WINDOW, &quarter);

    // f1 and f2: implied 120k against observed 140k.
    assert_eq!(implied.matched, 2);
    assert_eq!(implied.matched_implied_sats, 120_000);
    assert_eq!(implied.matched_observed_sats, 140_000);
    let ratio = implied.implied_vs_observed().expect("a ratio");
    assert!((ratio - (120_000.0 / 140_000.0 - 1.0)).abs() < 1e-12);
}

#[test]
fn the_assumed_share_is_looked_up_per_instance() {
    let data = DevFeeData {
        fees: vec![
            fee("a", 100, None),
            Fee {
                pubkey: "beta".to_string(),
                ..fee("b", 100, None)
            },
        ],
        settlements: vec![],
    };
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

    let data = DevFeeData {
        fees: vec![Fee {
            fee_in_force: None,
            ..fee("f", 100, None)
        }],
        settlements: vec![],
    };
    let stuck = summarise(&data, WINDOW, &quarter);
    assert_eq!(stuck.volume_sats, None);
    assert_eq!(stuck.not_invertible, 1);
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
    assert_eq!(metrics[1].value, Value::Sats(140_000));
    assert!(metrics[2].is_inferred());
    assert!(matches!(metrics[2].value, Value::Ratio(_)));
}

#[test]
fn the_error_column_states_the_bound_the_assumption_and_what_was_left_out() {
    let metrics = metrics("dev_fees", &summarise(&dataset(), WINDOW, &quarter));
    let error = metrics[0].error().expect("error");

    assert!(error.contains("±1200 sats"), "{error}");
    assert!(error.contains("pct assumed 0.25"), "{error}");
    assert!(error.contains("2 fees not invertible"), "{error}");
}

#[test]
fn a_ratio_over_no_matched_order_is_missing() {
    let data = DevFeeData {
        fees: vec![fee("orphan", 100, None)],
        settlements: vec![],
    };

    let metrics = metrics("dev_fees", &summarise(&data, WINDOW, &quarter));

    assert_eq!(metrics[2].value, Value::Missing);
}
