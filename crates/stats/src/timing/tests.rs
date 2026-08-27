//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md`
//! §12) for the timing figures of §6.4 and the funnel of §7.

use super::*;
use crate::activity::{Direction, Origin, Status};

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 1_900;

/// An order seen from its `pending` version at `pending_at`.
fn order(id: &str, pending_at: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at: pending_at,
        status: Status::Pending,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec!["cash".into()],
        amount_sats: 10_000,
        fiat_amount: Some(50.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(pending_at),
        origin: Origin {
            fiat_code: "ARS".into(),
            payment_methods: vec!["cash".into()],
            direction: Direction::Buy,
        },
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: Some(pending_at + 86_400),
    }
}

fn taken(taken_at: i64, order: Order) -> Order {
    Order {
        status: Status::InProgress,
        taken_at: Some(taken_at),
        ..order
    }
}

fn completed(taken_at: i64, success_at: i64, order: Order) -> Order {
    Order {
        status: Status::Success,
        success_at: Some(success_at),
        ..taken(taken_at, order)
    }
}

fn canceled(canceled_at: i64, order: Order) -> Order {
    Order {
        status: Status::Canceled,
        canceled_at: Some(canceled_at),
        ..order
    }
}

/// An order whose first version seen was not `pending`.
fn mid_flight(id: &str, first_seen: i64) -> Order {
    Order {
        pending_at: None,
        created_at: first_seen,
        expires_at: Some(first_seen + 86_400),
        ..order(id, first_seen)
    }
}

/// In the window `[1000, 2000)`, `now = 1900`:
///
/// - a: pending 1000, taken 1100, success 1400 → fill **100**, complete
///   **300**, cycle **400**
/// - b: pending 1050, taken 1350, success 1450 → fill **300**, complete
///   **100**, cycle **400**
/// - c: pending 1200, taken 1250, still in progress → fill **50**
/// - d: pending 1300, canceled 1500, no taker seen → cancel **200**
/// - e: pending 1400, taken 1500, canceled 1700 → fill **100**, cancel
///   **300**, canceled after a taker
/// - f: pending 1600, expires 1600 + 86400 → on the book at 1900, age
///   **300**
/// - g: pending 1700, expires 1800 → expired untaken at 1900
/// - regress: pending 1050, success 1120, canceled 1130 → the success is
///   canonical: cycle **70**, completed; **regressed**
/// - mid: first seen in progress at 1150, success 1250 → complete **100**
///   and nothing anchored on the book; **unknown origin**
/// - late: first seen at its success, 1650 → **unknown origin**, no gap
/// - old: pending 500, taken 900, success 1100 → fill outside the window,
///   complete **200** and cycle **600** inside it
/// - before: pending 100, canceled 200 → outside entirely
///
/// Fills [100, 300, 50, 100] → p50 **100**, p90 **300**, 4 samples.
/// Completes [300, 100, 100, 200] → p50 **100**, p90 **300**, 4 samples.
/// Cycles [400, 400, 70, 600] → p50 **400**, p90 **600**, 4 samples.
/// Cancels [200, 300] → p50 **200**, p90 **300**, 2 samples.
///
/// Funnel over the 8 whose pending was seen in the window (a–g, regress):
/// taken 5 (a, b, c, e, regress — a success implies a taker) → **5/8**;
/// completed 3; canceled after a taker 1 (e); canceled untaken 1 (d) →
/// **1/8**; expired untaken 1 (g); open 2 (c, f).
fn dataset() -> Vec<Order> {
    vec![
        completed(1_100, 1_400, order("a", 1_000)),
        completed(1_350, 1_450, order("b", 1_050)),
        taken(1_250, order("c", 1_200)),
        canceled(1_500, order("d", 1_300)),
        canceled(1_700, taken(1_500, order("e", 1_400))),
        order("f", 1_600),
        Order {
            expires_at: Some(1_800),
            ..order("g", 1_700)
        },
        Order {
            success_at: Some(1_120),
            canceled_at: Some(1_130),
            status: Status::Canceled,
            ..order("regress", 1_050)
        },
        completed(1_150, 1_250, mid_flight("mid", 1_150)),
        Order {
            status: Status::Success,
            success_at: Some(1_650),
            ..mid_flight("late", 1_650)
        },
        completed(900, 1_100, order("old", 500)),
        canceled(200, order("before", 100)),
    ]
}

#[test]
fn time_to_fill_is_from_the_pending_seen_to_the_taker_over_orders_taken_in_the_window() {
    // Arrange / Act
    let timing = summarise(&dataset(), WINDOW, NOW);

    // Assert
    assert_eq!(timing.time_to_fill_samples, 4);
    assert_eq!(timing.time_to_fill_p50, Some(100));
    assert_eq!(timing.time_to_fill_p90, Some(300));
}

#[test]
fn time_to_complete_needs_the_taker_and_full_cycle_needs_the_pending_each_with_its_own_count() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    // `mid` completes but has no cycle; `regress` cycles.
    assert_eq!(timing.time_to_complete_samples, 4);
    assert_eq!(timing.time_to_complete_p50, Some(100));
    assert_eq!(timing.time_to_complete_p90, Some(300));
    assert_eq!(timing.full_cycle_samples, 4);
    assert_eq!(timing.full_cycle_p50, Some(400));
    assert_eq!(timing.full_cycle_p90, Some(600));
}

#[test]
fn time_to_cancel_counts_only_the_canonical_cancellations() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.time_to_cancel_samples, 2);
    assert_eq!(timing.time_to_cancel_p50, Some(200));
    assert_eq!(timing.time_to_cancel_p90, Some(300));
}

#[test]
fn the_book_is_the_live_pending_orders_seen_from_their_entry() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.book_size, 1);
    assert_eq!(timing.book_age_avg, Some(300));
}

#[test]
fn the_funnel_is_over_the_orders_whose_pending_was_seen_in_the_window() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.funnel.created, 8);
    assert_eq!(timing.funnel.taken, 5);
    assert_eq!(timing.funnel.completed, 3);
    assert_eq!(timing.funnel.canceled_taken, 1);
    assert_eq!(timing.funnel.canceled_untaken, 1);
    assert_eq!(timing.funnel.expired_untaken, 1);
    assert_eq!(timing.funnel.open, 2);
    assert!((timing.funnel.taken_share().expect("share") - 5.0 / 8.0).abs() < 1e-12);
    assert!((timing.funnel.canceled_untaken_share().expect("share") - 1.0 / 8.0).abs() < 1e-12);
    let accounted = timing.funnel.completed
        + timing.funnel.canceled_taken
        + timing.funnel.canceled_untaken
        + timing.funnel.expired_untaken
        + timing.funnel.open;
    assert_eq!(
        accounted, timing.funnel.created,
        "every order ends up somewhere"
    );
}

#[test]
fn orders_first_seen_past_their_pending_are_of_unknown_origin_not_of_a_cohort() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.unknown_origin, 2);
    // `late` is a success seen in the window: not created, not completed,
    // not taken — it is in no cohort.
    let only_late = [dataset()
        .into_iter()
        .find(|o| o.order_id == "late")
        .unwrap()];
    let timing = summarise(&only_late, WINDOW, NOW);
    assert_eq!(timing.funnel, Funnel::default());
    assert_eq!(timing.unknown_origin, 1);
    assert_eq!(timing.full_cycle_samples, 0);
}

#[test]
fn a_success_then_a_cancellation_counts_once_as_the_earlier_end() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.regressed, 1);
    // The reverse order: canceled first, then a success — canceled wins.
    let reversed = [Order {
        canceled_at: Some(1_120),
        success_at: Some(1_130),
        status: Status::Success,
        ..order("r", 1_050)
    }];
    let timing = summarise(&reversed, WINDOW, NOW);
    assert_eq!(timing.funnel.canceled_untaken, 1);
    assert_eq!(timing.funnel.completed, 0);
    assert_eq!(timing.time_to_cancel_samples, 1);
    assert_eq!(timing.full_cycle_samples, 0);
}

#[test]
fn a_pending_order_past_its_expiry_is_expired_not_open() {
    let orders = [Order {
        expires_at: Some(NOW - 1),
        ..order("x", 1_500)
    }];

    let timing = summarise(&orders, WINDOW, NOW);

    assert_eq!(timing.funnel.expired_untaken, 1);
    assert_eq!(timing.funnel.open, 0);
    assert_eq!(timing.book_size, 0);
}

#[test]
fn an_order_from_the_future_or_with_an_absurd_clock_is_not_on_the_book() {
    let orders = [
        // Published after now: not yet on any book.
        order("future", NOW + 500),
        // A clock nobody has: the age would overflow.
        Order {
            expires_at: Some(i64::MAX),
            ..order("absurd", i64::MIN)
        },
        order("fine", 1_600),
    ];

    let timing = summarise(&orders, WINDOW, NOW);

    assert_eq!(timing.book_size, 1, "only the sane one is on the book");
    assert_eq!(timing.book_age_avg, Some(300));
}

#[test]
fn a_gap_that_overflows_or_runs_backwards_is_not_a_duration() {
    let orders = [
        taken(1_100, order("backwards", 1_150)),
        Order {
            taken_at: Some(1_100),
            ..order("overflow", i64::MIN)
        },
        taken(1_100, order("zero", 1_100)),
    ];

    let timing = summarise(&orders, WINDOW, NOW);

    assert_eq!(timing.time_to_fill_samples, 0);
}

#[test]
fn an_empty_window_has_no_durations_and_an_empty_funnel() {
    let timing = summarise(&dataset(), Window::new(5_000, 6_000), NOW);

    assert_eq!(timing.time_to_fill_samples, 0);
    assert_eq!(timing.time_to_fill_p50, None);
    assert_eq!(timing.funnel, Funnel::default());
    assert_eq!(timing.unknown_origin, 0);
    // The book is about now, not the window.
    assert_eq!(timing.book_size, 1);
}

#[test]
fn the_global_report_names_the_figures_in_order_all_observed() {
    let metrics = report(&dataset(), WINDOW, None, NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "timing.time_to_fill_samples",
            "timing.time_to_fill_p50",
            "timing.time_to_fill_p90",
            "timing.time_to_complete_samples",
            "timing.time_to_complete_p50",
            "timing.time_to_complete_p90",
            "timing.full_cycle_samples",
            "timing.full_cycle_p50",
            "timing.full_cycle_p90",
            "timing.time_to_cancel_samples",
            "timing.time_to_cancel_p50",
            "timing.time_to_cancel_p90",
            "timing.book_size",
            "timing.book_age_avg",
            "timing.funnel.created",
            "timing.funnel.taken",
            "timing.funnel.taken_share",
            "timing.funnel.completed",
            "timing.funnel.canceled_taken",
            "timing.funnel.canceled_untaken",
            "timing.funnel.canceled_untaken_share",
            "timing.funnel.expired_untaken",
            "timing.funnel.open",
            "timing.unknown_origin",
            "timing.regressed",
        ]
    );
    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn durations_are_seconds_shares_are_ratios_and_missing_is_missing() {
    let metrics = report(&dataset(), WINDOW, None, NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("timing.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("time_to_fill_p50"), &Value::Seconds(100));
    assert_eq!(value("book_age_avg"), &Value::Seconds(300));
    assert!(matches!(value("funnel.taken_share"), Value::Ratio(_)));
    assert_eq!(value("funnel.created"), &Value::Count(8));

    let empty = report(&dataset(), Window::new(5_000, 6_000), None, NOW);
    let missing = empty
        .iter()
        .find(|metric| metric.name == "timing.time_to_fill_p50")
        .expect("present");
    assert_eq!(missing.value, Value::Missing);
}

#[test]
fn slices_are_by_the_first_version_seen_not_the_latest() {
    // Entered the book as ARS/cash/buy, republished at success as
    // USD/bank/sell: its fill and cycle stay where they started.
    let moved = Order {
        fiat_code: "USD".into(),
        payment_methods: vec!["bank".into()],
        direction: Direction::Sell,
        ..completed(1_100, 1_400, order("moved", 1_000))
    };
    let orders = [moved];

    let by_fiat = report(&orders, WINDOW, Some(Dimension::Fiat), NOW);
    assert_eq!(by_fiat[0].name, "timing.ARS.time_to_fill_samples");
    assert_eq!(by_fiat[0].value, Value::Count(1));
    assert!(
        !by_fiat
            .iter()
            .any(|metric| metric.name.starts_with("timing.USD"))
    );

    let by_method = report(&orders, WINDOW, Some(Dimension::Method), NOW);
    assert_eq!(by_method[0].name, "timing.cash.time_to_fill_samples");

    let by_kind = report(&orders, WINDOW, Some(Dimension::Kind), NOW);
    assert_eq!(by_kind[0].name, "timing.buy.time_to_fill_samples");

    let by_instance = report(&orders, WINDOW, Some(Dimension::Instance), NOW);
    assert_eq!(
        by_instance[0].name,
        "timing.Alpha (pk).time_to_fill_samples"
    );
}
