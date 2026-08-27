//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md` §12).
//!
//! The window is `[1000, 2000)`. Fees are dated by their own event and
//! settlements by `success_at`, so the two lists are placed independently.

use super::*;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};

fn fee(id: &str, order_id: &str, created_at: i64, amount_sats: i64) -> Fee {
    Fee {
        event_id: id.to_string(),
        order_id: order_id.to_string(),
        instance: "Alpha (aaaaaaaa)".to_string(),
        created_at,
        amount_sats,
        is_duplicate: false,
        order_known: true,
        settled_at: None,
    }
}

fn settlement(order_id: &str, success_at: i64, has_fee: bool) -> Settlement {
    Settlement {
        order_id: order_id.to_string(),
        instance: "Alpha (aaaaaaaa)".to_string(),
        success_at,
        has_fee,
        charges_fee: Some(true),
    }
}

/// In the window `[1000, 2000)`:
///
/// - canonical fees in the window: f1 (100 sats, latency 50), f2 (200 sats,
///   latency 350), f3 (300 sats, orphan), f5 (400 sats, canonical of a
///   doubly-paid order, latency 100) → **paid 4, total 1000**
/// - f4 is a duplicate of f5: excluded from the total, its order counted
///   once under `duplicates` → **1**
/// - f6 is before the window, f7 after: ignored
/// - orphans: f3 → **1**
/// - latencies 50, 350, 100 → p50 **100**, p90 **350**
/// - settlements in the window: s1 (fee), s2 (fee), s3 (no fee), s4 (no
///   fee, instance charges nothing → not owed) → coverage **2/3**;
///   s5 is outside the window
fn dataset() -> DevFeeData {
    DevFeeData {
        fees: vec![
            Fee {
                settled_at: Some(1_050),
                ..fee("f1", "o1", 1_100, 100)
            },
            Fee {
                settled_at: Some(1_150),
                ..fee("f2", "o2", 1_500, 200)
            },
            Fee {
                order_known: false,
                ..fee("f3", "unseen", 1_600, 300)
            },
            Fee {
                is_duplicate: true,
                settled_at: Some(1_700),
                ..fee("f4", "o5", 1_900, 400)
            },
            Fee {
                settled_at: Some(1_700),
                ..fee("f5", "o5", 1_800, 400)
            },
            fee("f6", "o6", 900, 999),
            fee("f7", "o7", 2_000, 999),
        ],
        settlements: vec![
            settlement("s1", 1_100, true),
            settlement("s2", 1_200, true),
            settlement("s3", 1_300, false),
            Settlement {
                charges_fee: Some(false),
                ..settlement("s4", 1_400, false)
            },
            settlement("s5", 2_500, false),
        ],
    }
}

#[test]
fn the_total_excludes_duplicates_and_fees_outside_the_window() {
    // Arrange / Act
    let dev_fees = summarise(&dataset(), WINDOW);

    // Assert
    assert_eq!(dev_fees.total_sats, 1_000);
    assert_eq!(dev_fees.paid, 4);
}

#[test]
fn a_doubly_paid_order_is_counted_once_as_a_duplicate() {
    let dev_fees = summarise(&dataset(), WINDOW);

    assert_eq!(dev_fees.duplicates, 1);
}

#[test]
fn a_fee_for_an_unseen_order_is_an_orphan() {
    let dev_fees = summarise(&dataset(), WINDOW);

    assert_eq!(dev_fees.orphans, 1);
}

#[test]
fn latency_is_measured_from_the_settlement_over_fees_whose_order_is_known() {
    let dev_fees = summarise(&dataset(), WINDOW);

    assert_eq!(dev_fees.latency_p50, Some(100));
    assert_eq!(dev_fees.latency_p90, Some(350));
}

#[test]
fn coverage_counts_only_the_settlements_that_owed_a_fee() {
    let dev_fees = summarise(&dataset(), WINDOW);

    // s1 and s2 paid, s3 did not; s4's instance charges nothing.
    assert_eq!(dev_fees.coverage, Some(2.0 / 3.0));
}

#[test]
fn an_instance_whose_fee_policy_is_unknown_is_left_out_of_coverage() {
    // Whether it owed a fee is not observable, and coverage is an observed
    // figure: guessing would understate it during a backfill that has not
    // yet reached the instance's 38385.
    let data = DevFeeData {
        fees: Vec::new(),
        settlements: vec![Settlement {
            charges_fee: None,
            ..settlement("s", 1_500, false)
        }],
    };

    assert_eq!(summarise(&data, WINDOW).coverage, None);
}

#[test]
fn coverage_counts_a_settlement_only_when_its_instance_is_known_to_charge() {
    let data = DevFeeData {
        fees: Vec::new(),
        settlements: vec![
            settlement("known", 1_500, true),
            Settlement {
                charges_fee: None,
                ..settlement("unknown", 1_600, false)
            },
            Settlement {
                charges_fee: Some(false),
                ..settlement("free", 1_700, false)
            },
        ],
    };

    assert_eq!(summarise(&data, WINDOW).coverage, Some(1.0));
}

#[test]
fn an_empty_window_reports_missing_rates_and_zero_counts() {
    let dev_fees = summarise(&dataset(), Window::new(5_000, 6_000));

    assert_eq!(dev_fees.total_sats, 0);
    assert_eq!(dev_fees.coverage, None);
    assert_eq!(dev_fees.latency_p50, None);
    assert_eq!(dev_fees, DevFees::default());
}

#[test]
fn slicing_by_instance_splits_fees_and_settlements_alike() {
    let mut data = dataset();
    data.fees.push(Fee {
        instance: "Beta (bbbbbbbb)".to_string(),
        ..fee("b1", "ob", 1_100, 50)
    });
    data.settlements.push(Settlement {
        instance: "Beta (bbbbbbbb)".to_string(),
        ..settlement("sb", 1_100, true)
    });

    let groups = by_instance(&data);

    assert_eq!(groups.len(), 2);
    assert_eq!(groups["Beta (bbbbbbbb)"].fees.len(), 1);
    assert_eq!(groups["Beta (bbbbbbbb)"].settlements.len(), 1);
    assert_eq!(summarise(&groups["Beta (bbbbbbbb)"], WINDOW).total_sats, 50);
}

#[test]
fn the_global_report_names_the_seven_figures_of_the_spec() {
    let names: Vec<String> = report(&dataset(), WINDOW, None)
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(
        names,
        vec![
            "dev_fees.total_sats",
            "dev_fees.paid",
            "dev_fees.coverage",
            "dev_fees.latency_p50",
            "dev_fees.latency_p90",
            "dev_fees.duplicates",
            "dev_fees.orphans",
        ]
    );
}

#[test]
fn a_sliced_report_puts_the_slice_key_in_the_name() {
    let by_instance = report(&dataset(), WINDOW, Some(Dimension::Instance));
    assert_eq!(by_instance[0].name, "dev_fees.Alpha (aaaaaaaa).total_sats");
    assert_eq!(by_instance[0].value, Value::Sats(1_000));

    // 2026-07-01 to 2026-09-01: two months, seven figures each.
    let by_month = report(
        &dataset(),
        Window::new(1_782_864_000, 1_788_220_800),
        Some(Dimension::Month),
    );
    assert_eq!(by_month.len(), 14);
    assert_eq!(by_month[0].name, "dev_fees.2026-07.total_sats");
    assert_eq!(by_month[7].name, "dev_fees.2026-08.total_sats");
}

#[test]
fn missing_figures_are_reported_as_missing_not_as_zero() {
    let metrics = report(&dataset(), Window::new(5_000, 6_000), None);
    let coverage = metrics
        .iter()
        .find(|metric| metric.name == "dev_fees.coverage")
        .expect("present");
    let latency = metrics
        .iter()
        .find(|metric| metric.name == "dev_fees.latency_p50")
        .expect("present");

    assert_eq!(coverage.value, Value::Missing);
    assert_eq!(latency.value, Value::Missing);
}

#[test]
fn every_dev_fee_metric_is_observed() {
    assert!(
        report(&dataset(), WINDOW, None)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}
