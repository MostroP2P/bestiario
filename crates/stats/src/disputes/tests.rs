//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md` §12).
//!
//! The window is `[1000, 2000)` and `now` is 2500.

use super::*;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 2_500;

fn dispute(id: &str, opened_at: i64, status: Status, initiator: Option<Initiator>) -> Dispute {
    Dispute {
        dispute_id: id.to_string(),
        instance: "Alpha (aaaaaaaa)".to_string(),
        opened_at,
        status,
        initiator,
        resolved_at: None,
    }
}

fn resolved(id: &str, opened_at: i64, status: Status, resolved_at: i64) -> Dispute {
    Dispute {
        resolved_at: Some(resolved_at),
        ..dispute(id, opened_at, status, Some(Initiator::Seller))
    }
}

fn taken(id: &str, left_pending_at: i64) -> Taken {
    Taken {
        order_id: id.to_string(),
        instance: "Alpha (aaaaaaaa)".to_string(),
        left_pending_at,
    }
}

/// In the window `[1000, 2000)`:
///
/// - opened: d1 (initiated, buyer), d2 (in-progress, buyer), d3 (settled,
///   seller, resolved at 1500), d4 (released, no initiator, resolved at
///   1900) → **4**; by status: initiated 1, in-progress 1, settled 1,
///   released 1
/// - initiator: 2 buyers of 3 known → buyer **2/3**
/// - orders that left pending in the window: t1, t2, t3, t4, t5 → rate
///   **4/5**
/// - resolved in the window: d3 (500s), d4 (600s), d5 (opened before the
///   window, seller-refunded at 1200, 700s) → **3**; outcome refunded 1/3,
///   settled 1/3, released 1/3; resolution p50 **600**, p90 **700**
/// - open now: d1, d2, d6 (opened after the window, still initiated) →
///   **3**; oldest is d1 at 1100 → age **1400**
fn dataset() -> DisputeData {
    DisputeData {
        disputes: vec![
            dispute("d1", 1_100, Status::Initiated, Some(Initiator::Buyer)),
            dispute("d2", 1_200, Status::InProgress, Some(Initiator::Buyer)),
            resolved("d3", 1_000, Status::Settled, 1_500),
            Dispute {
                initiator: None,
                ..resolved("d4", 1_300, Status::Released, 1_900)
            },
            resolved("d5", 500, Status::SellerRefunded, 1_200),
            dispute("d6", 2_100, Status::Initiated, Some(Initiator::Seller)),
            // Resolved after the window: opened before it, so counted nowhere.
            resolved("d7", 900, Status::Settled, 2_200),
        ],
        taken: vec![
            taken("t1", 1_000),
            taken("t2", 1_100),
            taken("t3", 1_200),
            taken("t4", 1_300),
            taken("t5", 1_999),
            taken("t6", 2_000),
        ],
    }
}

#[test]
fn opened_is_dated_by_the_opening_tag_and_split_by_current_status() {
    // Arrange / Act
    let disputes = summarise(&dataset(), WINDOW, NOW);

    // Assert
    assert_eq!(disputes.opened, 4);
    assert_eq!(disputes.by_status, [1, 1, 0, 1, 1]);
}

#[test]
fn the_initiator_share_ignores_disputes_that_did_not_say() {
    let disputes = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(disputes.buyer_share, Some(2.0 / 3.0));
}

#[test]
fn the_rate_divides_openings_by_orders_that_found_a_taker_in_the_window() {
    let disputes = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(disputes.rate, Some(4.0 / 5.0));
}

#[test]
fn outcome_and_resolution_time_are_dated_by_the_terminal_version() {
    let disputes = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(disputes.resolved, 3);
    assert_eq!(disputes.outcome, Some([1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]));
    assert_eq!(disputes.resolution_p50, Some(600));
    assert_eq!(disputes.resolution_p90, Some(700));
}

#[test]
fn open_now_counts_every_non_terminal_dispute_whenever_it_was_opened() {
    let disputes = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(disputes.open_now, 3);
    assert_eq!(disputes.open_oldest_age, Some(NOW - 1_100));
}

#[test]
fn an_empty_window_reports_missing_rates_not_zero_ones() {
    let disputes = summarise(&dataset(), Window::new(5_000, 6_000), NOW);

    assert_eq!(disputes.opened, 0);
    assert_eq!(disputes.buyer_share, None);
    assert_eq!(disputes.rate, None);
    assert_eq!(disputes.outcome, None);
    assert_eq!(disputes.resolution_p50, None);
    // The "now" figures do not depend on the window.
    assert_eq!(disputes.open_now, 3);
}

#[test]
fn no_open_disputes_means_no_oldest_age() {
    let data = DisputeData {
        disputes: vec![resolved("d", 1_000, Status::Settled, 1_100)],
        taken: Vec::new(),
    };

    assert_eq!(summarise(&data, WINDOW, NOW).open_oldest_age, None);
}

#[test]
fn slicing_by_instance_splits_disputes_and_taken_orders_alike() {
    let mut data = dataset();
    data.disputes.push(Dispute {
        instance: "Beta (bbbbbbbb)".to_string(),
        ..dispute("b1", 1_100, Status::Initiated, None)
    });
    data.taken.push(Taken {
        instance: "Beta (bbbbbbbb)".to_string(),
        ..taken("tb", 1_100)
    });

    let groups = by_instance(&data);

    assert_eq!(groups.len(), 2);
    assert_eq!(
        summarise(&groups["Beta (bbbbbbbb)"], WINDOW, NOW).rate,
        Some(1.0)
    );
}

#[test]
fn the_global_report_names_every_figure_of_the_spec() {
    let names: Vec<String> = report(&dataset(), WINDOW, NOW, None)
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(
        names,
        vec![
            "disputes.opened",
            "disputes.status.initiated",
            "disputes.status.in_progress",
            "disputes.status.seller_refunded",
            "disputes.status.settled",
            "disputes.status.released",
            "disputes.initiator.buyer",
            "disputes.initiator.seller",
            "disputes.rate",
            "disputes.resolved",
            "disputes.outcome.seller_refunded",
            "disputes.outcome.settled",
            "disputes.outcome.released",
            "disputes.resolution_p50",
            "disputes.resolution_p90",
            "disputes.open_now",
            "disputes.open_oldest_age",
        ]
    );
}

#[test]
fn the_seller_share_is_the_complement_of_the_buyer_share() {
    let metrics = report(&dataset(), WINDOW, NOW, Some(Dimension::Initiator));

    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].name, "disputes.initiator.buyer");
    assert_eq!(metrics[0].value, Value::Ratio(2.0 / 3.0));
    assert_eq!(metrics[1].value, Value::Ratio(1.0 - 2.0 / 3.0));
}

#[test]
fn the_status_histogram_is_five_counts() {
    let metrics = report(&dataset(), WINDOW, NOW, Some(Dimension::Status));

    assert_eq!(metrics.len(), 5);
    assert_eq!(metrics[0].name, "disputes.status.initiated");
    assert_eq!(metrics[0].value, Value::Count(1));
}

#[test]
fn a_monthly_report_leaves_the_now_figures_out() {
    // 2026-07-01 to 2026-09-01: two months, fifteen dated figures each.
    let names: Vec<String> = report(
        &dataset(),
        Window::new(1_782_864_000, 1_788_220_800),
        NOW,
        Some(Dimension::Month),
    )
    .into_iter()
    .map(|metric| metric.name)
    .collect();

    assert_eq!(names.len(), 30);
    assert_eq!(names[0], "disputes.2026-07.opened");
    assert_eq!(names[15], "disputes.2026-08.opened");
    assert!(
        names.iter().all(|name| !name.contains("open_")),
        "{names:?}"
    );
}

#[test]
fn an_instance_slice_keeps_the_now_figures() {
    let metrics = report(&dataset(), WINDOW, NOW, Some(Dimension::Instance));

    assert_eq!(metrics.len(), 17);
    assert_eq!(metrics[0].name, "disputes.Alpha (aaaaaaaa).opened");
    assert_eq!(
        metrics[16].name,
        "disputes.Alpha (aaaaaaaa).open_oldest_age"
    );
}

#[test]
fn every_dispute_metric_is_observed() {
    assert!(
        report(&dataset(), WINDOW, NOW, None)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}
