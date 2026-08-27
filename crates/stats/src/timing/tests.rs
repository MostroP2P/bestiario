//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md`
//! §12) for the timing figures of §6.4 and the funnel of §7.

use super::*;
use crate::activity::Direction;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 1_900;

fn order(id: &str, created_at: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at,
        status: Status::Pending,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec!["cash".into()],
        amount_sats: 10_000,
        fiat_amount: Some(50.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: Some(created_at + 86_400),
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

/// In the window `[1000, 2000)`, `now = 1900`:
///
/// - a: created 1000, taken 1100, completed 1400 → fill **100**, complete
///   **300**, cycle **400**
/// - b: created 1050, taken 1350, completed 1450 → fill **300**, complete
///   **100**, cycle **400**
/// - c: created 1200, taken 1250, still in progress → fill **50**
/// - d: created 1300, canceled untaken 1500 → cancel **200**
/// - e: created 1400, taken 1500, canceled 1700 → fill **100**, cancel
///   **300**, canceled after a taker
/// - f: created 1600, pending, expires 1600 + 86400 → live at 1900, age
///   **300**
/// - g: created 1700, pending, expired at 1800 → not live, still counted
///   as open in the funnel (no version says otherwise)
/// - old: created 500, taken 900, completed 1100 → fill outside the window
///   (900), complete **200** and cycle **600** inside it
/// - before: created 100, canceled 200 → outside entirely
///
/// Fills in the window [100, 300, 50, 100] → sorted [50, 100, 100, 300]:
/// p50 **100**, p90 **300**. Completes [300, 100, 200] → p50 **200**, p90
/// **300**. Cycles [400, 400, 600] → p50 **400**, p90 **600**. Cancels
/// [200, 300] → p50 **200**, p90 **300**.
///
/// Funnel over the 7 created in the window (a–g): taken 4 (a, b, c, e) →
/// **4/7**; canceled untaken 1 (d) → **1/7**; canceled after a taker 1
/// (e); completed 2; open 3 (c still in progress, f, g).
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
        completed(900, 1_100, order("old", 500)),
        canceled(200, order("before", 100)),
    ]
}

#[test]
fn time_to_fill_is_over_the_orders_taken_in_the_window() {
    // Arrange / Act
    let timing = summarise(&dataset(), WINDOW, NOW);

    // Assert
    assert_eq!(timing.filled, 4);
    assert_eq!(timing.time_to_fill_p50, Some(100));
    assert_eq!(timing.time_to_fill_p90, Some(300));
}

#[test]
fn time_to_complete_and_full_cycle_are_over_the_orders_completed_in_the_window() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.completed, 3);
    assert_eq!(timing.time_to_complete_p50, Some(200));
    assert_eq!(timing.time_to_complete_p90, Some(300));
    assert_eq!(timing.full_cycle_p50, Some(400));
    assert_eq!(timing.full_cycle_p90, Some(600));
}

#[test]
fn time_to_cancel_is_over_the_orders_canceled_in_the_window() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.canceled, 2);
    assert_eq!(timing.time_to_cancel_p50, Some(200));
    assert_eq!(timing.time_to_cancel_p90, Some(300));
}

#[test]
fn book_age_is_the_mean_age_of_the_live_pending_orders_now() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.book_size, 1);
    assert_eq!(timing.book_age_avg, Some(300));
}

#[test]
fn the_funnel_is_over_the_orders_created_in_the_window() {
    let timing = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(timing.funnel.created, 7);
    assert_eq!(timing.funnel.taken, 4);
    assert_eq!(timing.funnel.canceled_untaken, 1);
    assert_eq!(timing.funnel.canceled_taken, 1);
    assert_eq!(timing.funnel.completed, 2);
    assert_eq!(timing.funnel.open, 3);
    assert!((timing.funnel.taken_share().expect("share") - 4.0 / 7.0).abs() < 1e-12);
    assert!((timing.funnel.canceled_untaken_share().expect("share") - 1.0 / 7.0).abs() < 1e-12);
}

#[test]
fn an_empty_window_has_no_durations_and_an_empty_funnel() {
    let timing = summarise(&dataset(), Window::new(5_000, 6_000), NOW);

    assert_eq!(timing.filled, 0);
    assert_eq!(timing.time_to_fill_p50, None);
    assert_eq!(timing.full_cycle_p90, None);
    assert_eq!(timing.time_to_cancel_p50, None);
    assert_eq!(timing.funnel.created, 0);
    assert_eq!(timing.funnel.taken_share(), None);
    // The book is about now, not the window.
    assert_eq!(timing.book_size, 1);
}

#[test]
fn a_taker_who_arrived_before_the_book_entry_is_not_a_negative_fill() {
    // A malformed history: in-progress published before pending.
    let orders = vec![Order {
        taken_at: Some(900),
        ..taken(900, order("odd", 1_100))
    }];

    let timing = summarise(&orders, WINDOW, NOW);

    assert_eq!(timing.filled, 0);
    assert_eq!(timing.time_to_fill_p50, None);
}

#[test]
fn an_order_first_seen_mid_flight_yields_no_gap_rather_than_a_zero() {
    // Caught already in progress at 1100: created_at is that same event.
    let mid_flight = taken(1_100, order("mid", 1_100));
    // Caught only at its cancellation.
    let gone = canceled(1_200, order("gone", 1_200));

    let timing = summarise(&[mid_flight, gone], WINDOW, NOW);

    assert_eq!(timing.filled, 0);
    assert_eq!(timing.canceled, 0);
    assert_eq!(timing.time_to_cancel_p50, None);
    // The funnel still knows what they became.
    assert_eq!(timing.funnel.taken, 1);
    assert_eq!(timing.funnel.canceled_untaken, 1);
}

#[test]
fn the_global_report_names_the_figures_in_order_all_observed() {
    let metrics = report(&dataset(), WINDOW, None, NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "timing.filled",
            "timing.time_to_fill_p50",
            "timing.time_to_fill_p90",
            "timing.completed",
            "timing.time_to_complete_p50",
            "timing.time_to_complete_p90",
            "timing.full_cycle_p50",
            "timing.full_cycle_p90",
            "timing.canceled",
            "timing.time_to_cancel_p50",
            "timing.time_to_cancel_p90",
            "timing.book_size",
            "timing.book_age_avg",
            "timing.funnel.created",
            "timing.funnel.taken",
            "timing.funnel.taken_share",
            "timing.funnel.canceled_untaken",
            "timing.funnel.canceled_untaken_share",
            "timing.funnel.canceled_taken",
            "timing.funnel.completed",
            "timing.funnel.open",
        ]
    );
    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn durations_are_seconds_and_shares_are_ratios() {
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
    assert_eq!(value("funnel.created"), &Value::Count(7));
}

#[test]
fn missing_durations_are_missing_not_zero() {
    let metrics = report(&dataset(), Window::new(5_000, 6_000), None, NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("timing.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("time_to_fill_p50"), &Value::Missing);
    assert_eq!(value("funnel.taken_share"), &Value::Missing);
    assert_eq!(value("filled"), &Value::Count(0));
}

#[test]
fn slices_put_the_key_in_the_name() {
    let by_fiat = report(&dataset(), WINDOW, Some(Dimension::Fiat), NOW);
    assert_eq!(by_fiat[0].name, "timing.ARS.filled");
    assert_eq!(by_fiat.len(), 21);

    let by_method = report(&dataset(), WINDOW, Some(Dimension::Method), NOW);
    assert_eq!(by_method[0].name, "timing.cash.filled");

    let by_kind = report(&dataset(), WINDOW, Some(Dimension::Kind), NOW);
    assert_eq!(by_kind[0].name, "timing.buy.filled");

    let by_instance = report(&dataset(), WINDOW, Some(Dimension::Instance), NOW);
    assert_eq!(by_instance[0].name, "timing.Alpha (pk).filled");
}
