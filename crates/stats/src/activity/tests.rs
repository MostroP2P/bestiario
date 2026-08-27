//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md` §12).
//!
//! The window is `[1000, 2000)`, the previous one `[0, 1000)`, and `now` is
//! 2500. Every order below is placed relative to those three numbers so the
//! arithmetic can be checked by reading the fixture.

use super::*;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 2_500;

/// An order with the given lifecycle, everything else defaulted.
fn order(id: &str, created_at: i64, status: Status) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk-a".into(),
        instance: "Alpha".into(),
        created_at,
        status,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec!["cash".into()],
        amount_sats: 1_000,
        fiat_amount: Some(50.0),
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: None,
    }
}

fn completed(id: &str, created_at: i64, taken_at: i64, success_at: i64) -> Order {
    Order {
        taken_at: Some(taken_at),
        success_at: Some(success_at),
        ..order(id, created_at, Status::Success)
    }
}

fn abandoned(id: &str, created_at: i64, canceled_at: i64) -> Order {
    Order {
        canceled_at: Some(canceled_at),
        ..order(id, created_at, Status::Canceled)
    }
}

fn canceled_after_taker(id: &str, created_at: i64, taken_at: i64, canceled_at: i64) -> Order {
    Order {
        taken_at: Some(taken_at),
        canceled_at: Some(canceled_at),
        ..order(id, created_at, Status::Canceled)
    }
}

fn pending(id: &str, created_at: i64, expires_at: i64) -> Order {
    Order {
        expires_at: Some(expires_at),
        ..order(id, created_at, Status::Pending)
    }
}

/// The dataset every summary test reads. In the window `[1000, 2000)`:
///
/// - created: c1, c2, c3, a1, t1, p1 → **6**
/// - completed (success_at in window): c1, c2, c4 → **3**
/// - canceled (canceled_at in window): a1, t1, a2 → **3**
/// - completion rate: 3 / (3 + 3) = **0.5**
/// - abandonment: of the 6 created, a1 died without a taker → **1/6**
/// - open now (pending, expires > 2500): p1 → **1** (p2 expired)
/// - in progress now: i1 → **1**
/// - previous window `[0, 1000)`: created c4, a2, p2, i1 → 4; completed: none
fn dataset() -> Vec<Order> {
    vec![
        completed("c1", 1_100, 1_200, 1_300),
        completed("c2", 1_400, 1_500, 1_900),
        // Created in the window, completed after it: counts as created only.
        completed("c3", 1_800, 1_900, 2_100),
        // Created before the window, completed inside it: counts as completed only.
        completed("c4", 500, 600, 1_050),
        abandoned("a1", 1_200, 1_250),
        // Created before the window, canceled inside it.
        abandoned("a2", 800, 1_100),
        canceled_after_taker("t1", 1_300, 1_350, 1_400),
        pending("p1", 1_600, 3_000),
        // Expired: pending but its expiry is behind the clock.
        pending("p2", 900, 2_000),
        Order {
            taken_at: Some(950),
            ..order("i1", 900, Status::InProgress)
        },
    ]
}

#[test]
fn created_counts_first_versions_inside_the_window() {
    // Arrange / Act
    let activity = summarise(&dataset(), WINDOW, NOW);

    // Assert
    assert_eq!(activity.created, 6);
}

#[test]
fn completed_and_canceled_are_dated_by_when_they_happened_not_by_creation() {
    let activity = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(
        activity.completed, 3,
        "c1, c2 and c4; c3 completed after the window"
    );
    assert_eq!(activity.canceled, 3, "a1, t1 and a2");
}

#[test]
fn completion_rate_is_completed_over_completed_plus_canceled() {
    let activity = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(activity.completion_rate, Some(0.5));
}

#[test]
fn abandonment_rate_is_the_share_of_the_created_cohort_that_died_without_a_taker() {
    let activity = summarise(&dataset(), WINDOW, NOW);

    // a1 only: t1 was canceled but had a taker; a2 was created before the window.
    assert_eq!(activity.abandonment_rate, Some(1.0 / 6.0));
}

#[test]
fn open_now_counts_pending_orders_that_have_not_expired() {
    let activity = summarise(&dataset(), WINDOW, NOW);

    assert_eq!(
        activity.open_now, 1,
        "p1; p2 expired at 2000 and now is 2500"
    );
    assert_eq!(activity.in_progress_now, 1);
}

#[test]
fn a_pending_order_with_no_expiry_is_not_reported_as_open() {
    // No expiry means the version did not say, and "open" is a claim the
    // data has to make.
    let orders = vec![order("p", 1_500, Status::Pending)];

    assert_eq!(summarise(&orders, WINDOW, NOW).open_now, 0);
}

#[test]
fn deltas_compare_against_the_previous_window_of_the_same_length() {
    let activity = summarise(&dataset(), WINDOW, NOW);

    // created: 6 now vs 4 before → +50%. completed: 3 now vs 0 before → no
    // base to grow from.
    assert_eq!(activity.created_delta, Some(0.5));
    assert_eq!(activity.completed_delta, None);
}

#[test]
fn a_window_with_nothing_in_it_reports_no_rates_rather_than_zero_ones() {
    let activity = summarise(&dataset(), Window::new(5_000, 6_000), NOW);

    assert_eq!(activity.created, 0);
    assert_eq!(activity.completion_rate, None);
    assert_eq!(activity.abandonment_rate, None);
    assert_eq!(activity.created_delta, None);
}

#[test]
fn slicing_by_method_puts_an_order_with_two_methods_in_both() {
    let orders = vec![
        Order {
            payment_methods: vec!["cash".into(), "bank".into()],
            ..order("x", 1_100, Status::Pending)
        },
        order("y", 1_200, Status::Pending),
    ];

    let groups = slice(&orders, Dimension::Method);

    assert_eq!(groups["cash"].len(), 2);
    assert_eq!(groups["bank"].len(), 1);
}

#[test]
fn slicing_by_instance_uses_the_label_the_loader_chose() {
    let orders = vec![
        order("x", 1_100, Status::Pending),
        Order {
            pubkey: "pk-b".into(),
            instance: "pk-b".into(),
            ..order("y", 1_200, Status::Pending)
        },
    ];

    let keys: Vec<String> = slice(&orders, Dimension::Instance).into_keys().collect();

    assert_eq!(keys, vec!["Alpha", "pk-b"]);
}

#[test]
fn the_hour_histogram_buckets_created_and_completed_separately() {
    // 1_000 is 00:16 UTC on 1970-01-01; 1_000 + 3_600 is 01:16.
    let orders = vec![
        completed("c", 1_000, 1_100, 1_000 + 3_600),
        order("p", 1_010, Status::Pending),
    ];

    let histogram = by_hour(&orders, Window::new(0, 10_000));

    assert_eq!(histogram.labels.len(), 24);
    assert_eq!(histogram.created[0], 2);
    assert_eq!(histogram.completed[0], 0);
    assert_eq!(histogram.completed[1], 1);
}

#[test]
fn the_weekday_histogram_starts_on_monday() {
    // 1970-01-01 was a Thursday.
    let orders = vec![order("p", 1_000, Status::Pending)];

    let histogram = by_weekday(&orders, Window::new(0, 10_000));

    assert_eq!(histogram.labels[0], "mon");
    assert_eq!(histogram.created[3], 1, "thursday");
}

#[test]
fn the_global_report_names_the_nine_figures_of_the_spec() {
    let names: Vec<String> = report(&dataset(), WINDOW, NOW, None)
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(
        names,
        vec![
            "orders.created",
            "orders.completed",
            "orders.canceled",
            "orders.completion_rate",
            "orders.abandonment_rate",
            "orders.created_delta",
            "orders.completed_delta",
            "orders.open_now",
            "orders.in_progress_now",
        ]
    );
}

#[test]
fn a_sliced_report_puts_the_slice_key_in_the_name() {
    let metrics = report(&dataset(), WINDOW, NOW, Some(Dimension::Kind));

    assert_eq!(metrics[0].name, "orders.buy.created");
    assert_eq!(metrics[0].value, Value::Count(6));
}

#[test]
fn a_missing_rate_is_reported_as_missing_not_as_zero() {
    let metrics = report(&dataset(), Window::new(5_000, 6_000), NOW, None);
    let rate = metrics
        .iter()
        .find(|metric| metric.name == "orders.completion_rate")
        .expect("present");

    assert_eq!(rate.value, Value::Missing);
}

#[test]
fn a_monthly_report_has_one_block_per_month_in_the_window() {
    // 2026-07-01 to 2026-09-01: two months.
    let window = Window::new(1_782_864_000, 1_788_220_800);
    let orders = vec![order("x", 1_782_864_000 + 86_400, Status::Pending)];

    let names: Vec<String> = report(&orders, window, NOW, Some(Dimension::Month))
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    // Seven per month: the two "now" figures are not dated and are left out.
    assert_eq!(names.len(), 14);
    assert_eq!(names[0], "orders.2026-07.created");
    assert_eq!(names[7], "orders.2026-08.created");
    assert!(
        names.iter().all(|name| !name.ends_with("_now")),
        "{names:?}"
    );
}

#[test]
fn every_activity_metric_is_observed() {
    // §6.1 is all counts of published events; nothing here is inferred, and
    // a stray `(inf)` would be a lie about the source.
    assert!(
        report(&dataset(), WINDOW, NOW, None)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}

#[test]
fn a_histogram_report_names_every_bucket_twice() {
    let names: Vec<String> = report(&dataset(), WINDOW, NOW, Some(Dimension::Hour))
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(names.len(), 48);
    assert_eq!(names[0], "orders.hour.00.created");
    assert_eq!(names[1], "orders.hour.00.completed");
    assert_eq!(names[47], "orders.hour.23.completed");

    let weekday = report(&dataset(), WINDOW, NOW, Some(Dimension::Weekday));
    assert_eq!(weekday.len(), 14);
    assert_eq!(weekday[0].name, "orders.weekday.mon.created");
}

#[test]
fn the_grouping_slicer_has_nothing_to_say_about_windows_or_histograms() {
    // Months are windows and the histograms are buckets; neither is a
    // group, and asking gives no groups rather than one wrong group.
    for dimension in [Dimension::Month, Dimension::Hour, Dimension::Weekday] {
        assert!(slice(&dataset(), dimension).is_empty(), "{dimension:?}");
    }
}

#[test]
fn slicing_by_status_fiat_and_method_each_name_the_slice() {
    let by_status = report(&dataset(), WINDOW, NOW, Some(Dimension::Status));
    let by_fiat = report(&dataset(), WINDOW, NOW, Some(Dimension::Fiat));
    let by_method = report(&dataset(), WINDOW, NOW, Some(Dimension::Method));

    assert_eq!(by_status[0].name, "orders.canceled.created");
    assert_eq!(by_fiat[0].name, "orders.ARS.created");
    assert_eq!(by_method[0].name, "orders.cash.created");
}

#[test]
fn a_monthly_delta_compares_against_the_calendar_month_before_not_the_same_number_of_seconds() {
    // 2026-03-01 and 2026-04-01. March has thirty-one days; the thirty-one
    // days before it start on January 29, and February has twenty-eight.
    let mar_1 = 1_772_323_200;
    let apr_1 = 1_775_001_600;
    let feb_1 = mar_1 - 28 * 86_400;
    let jan_30 = feb_1 - 2 * 86_400;

    let orders = vec![
        order("march", mar_1 + 86_400, Status::Pending),
        order("february", feb_1 + 86_400, Status::Pending),
        // Inside the thirty-one seconds-window before March, outside February.
        order("january", jan_30, Status::Pending),
    ];

    let march = report(
        &orders,
        Window::new(mar_1, apr_1),
        NOW,
        Some(Dimension::Month),
    )
    .into_iter()
    .find(|metric| metric.name == "orders.2026-03.created_delta")
    .expect("present");

    // One created in March against one in February: no growth. The
    // same-length window would have counted January 30 and reported −50%.
    assert_eq!(march.value, Value::Ratio(0.0));
}

#[test]
fn a_summary_with_no_previous_window_reports_no_deltas() {
    let activity = summarise_against(&dataset(), WINDOW, None, NOW);

    assert_eq!(activity.created, 6);
    assert_eq!(activity.created_delta, None);
    assert_eq!(activity.completed_delta, None);
}
