use super::*;

// 2026-07-01T00:00:00Z and 2026-09-01T00:00:00Z.
const JUL_1: i64 = 1_782_864_000;
const AUG_1: i64 = 1_785_542_400;
const SEP_1: i64 = 1_788_220_800;

#[test]
fn a_window_is_closed_at_the_start_and_open_at_the_end() {
    let window = Window::new(100, 200);

    assert!(window.contains(100));
    assert!(window.contains(199));
    assert!(!window.contains(200));
    assert!(!window.contains(99));
}

#[test]
fn the_previous_window_has_the_same_length_and_ends_where_this_one_starts() {
    assert_eq!(Window::new(100, 250).previous(), Window::new(-50, 100));
}

#[test]
fn a_window_covering_two_whole_months_yields_both_unclipped() {
    let months = Window::new(JUL_1, SEP_1).months();

    assert_eq!(
        months,
        vec![
            ("2026-07".to_string(), Window::new(JUL_1, AUG_1)),
            ("2026-08".to_string(), Window::new(AUG_1, SEP_1)),
        ]
    );
}

#[test]
fn a_window_opening_mid_month_clips_that_month_to_where_it_opened() {
    let mid_july = JUL_1 + 14 * 86_400;
    let mid_august = AUG_1 + 10 * 86_400;

    let months = Window::new(mid_july, mid_august).months();

    assert_eq!(
        months,
        vec![
            ("2026-07".to_string(), Window::new(mid_july, AUG_1)),
            ("2026-08".to_string(), Window::new(AUG_1, mid_august)),
        ]
    );
}

#[test]
fn december_rolls_over_into_the_next_year() {
    // 2026-12-01 and 2027-02-01.
    let dec_1 = 1_796_083_200;
    let feb_1 = 1_801_440_000;

    let keys: Vec<String> = Window::new(dec_1, feb_1)
        .months()
        .into_iter()
        .map(|(key, _)| key)
        .collect();

    assert_eq!(keys, vec!["2026-12", "2027-01"]);
}

#[test]
fn a_whole_month_is_preceded_by_the_whole_month_before_it() {
    // June has thirty days and July thirty-one: `previous()` would have
    // started on May 31.
    let jun_1 = 1_780_272_000;

    assert_eq!(
        Window::new(JUL_1, AUG_1).previous_month(),
        Some(Window::new(jun_1, JUL_1))
    );
}

#[test]
fn a_clipped_month_is_preceded_by_the_same_days_of_the_month_before() {
    // [Aug 1, Aug 27) → [Jul 1, Jul 27).
    let aug_27 = AUG_1 + 26 * 86_400;
    let jul_27 = JUL_1 + 26 * 86_400;

    assert_eq!(
        Window::new(AUG_1, aug_27).previous_month(),
        Some(Window::new(JUL_1, jul_27))
    );
}

#[test]
fn a_day_the_earlier_month_does_not_have_clamps_to_its_last_one() {
    // [Mar 31, Apr 1) 2026 → [Feb 28, Mar 1): 2026 is not a leap year.
    let mar_31 = 1_774_915_200;
    let apr_1 = mar_31 + 86_400;
    let feb_28 = 1_772_236_800;
    let mar_1 = feb_28 + 86_400;

    assert_eq!(
        Window::new(mar_31, apr_1).previous_month(),
        Some(Window::new(feb_28, mar_1))
    );
}

#[test]
fn a_window_before_the_epoch_of_dates_has_no_previous_month() {
    assert_eq!(Window::new(i64::MIN, i64::MIN + 1).previous_month(), None);
}

/// 2026-08-27T00:00:00Z is a Thursday.
const THURSDAY: i64 = 1_787_788_800;
const DAY: i64 = 86_400;

#[test]
fn days_are_calendar_days_clipped_to_the_window() {
    // Arrange: midday Thursday to midday Saturday.
    let window = Window::new(THURSDAY + DAY / 2, THURSDAY + 2 * DAY + DAY / 2);

    // Act
    let days = window.days();

    // Assert
    let keys: Vec<&str> = days.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["2026-08-27", "2026-08-28", "2026-08-29"]);
    assert_eq!(days[0].1.from, window.from, "the first day is clipped");
    assert_eq!(days[0].1.until, THURSDAY + DAY);
    assert_eq!(days[2].1.until, window.until, "and so is the last");
}

#[test]
fn weeks_open_on_monday_whichever_day_the_window_does() {
    let window = Window::new(THURSDAY, THURSDAY + 8 * DAY);

    let weeks = window.weeks();

    let keys: Vec<&str> = weeks.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["2026-W35", "2026-W36"]);
    assert_eq!(
        weeks[0].1.from, THURSDAY,
        "the week began on Monday; the window did not"
    );
    // The second bucket is a whole week from its Monday.
    assert_eq!(weeks[1].1.from, THURSDAY + 4 * DAY);
}

#[test]
fn years_are_calendar_years() {
    // 2025-12-31 to 2026-01-02.
    let new_year = 1_767_225_600;
    let window = Window::new(new_year - DAY, new_year + DAY);

    let years = window.years();

    let keys: Vec<&str> = years.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["2025", "2026"]);
    assert_eq!(years[1].1.from, new_year);
}

#[test]
fn the_buckets_tile_the_window_exactly() {
    let window = Window::new(THURSDAY + 1_000, THURSDAY + 20 * DAY - 7);

    for period in [Period::Day, Period::Week, Period::Month, Period::Year] {
        let buckets = window.buckets(period);
        assert_eq!(
            buckets[0].1.from, window.from,
            "{period:?} opens the window"
        );
        assert_eq!(
            buckets.last().expect("a bucket").1.until,
            window.until,
            "{period:?} closes it"
        );
        for pair in buckets.windows(2) {
            assert_eq!(pair[0].1.until, pair[1].1.from, "{period:?} leaves no gap");
        }
    }
}

#[test]
fn a_window_of_no_length_has_no_buckets() {
    let empty = Window::new(THURSDAY, THURSDAY);

    for period in [Period::Day, Period::Week, Period::Month, Period::Year] {
        assert!(empty.buckets(period).is_empty(), "{period:?}");
    }
}

// The two ends of the calendar. A window is as wide as the caller typed,
// and `Window` is a value type with no CLI in front of it.

#[test]
fn a_window_reaching_past_the_last_representable_date_is_bucketed_not_refused() {
    // Arrange: a window whose end an `i64` can name and the calendar
    // cannot, so the walk runs out of dates before it runs out of window.
    let ceiling = chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp();
    let window = Window::new(ceiling - 2 * DAY, i64::MAX);

    // Act
    let days = window.days();

    // Assert: the walk ends at the calendar's end rather than panicking,
    // and reports the buckets that do exist.
    assert!(!days.is_empty(), "the days before the end are still days");
    assert!(days.iter().all(|(_, bucket)| bucket.until <= ceiling + DAY));
}

#[test]
fn a_limit_is_still_refused_over_a_window_wider_than_it_allows() {
    let window = Window::new(0, 40 * DAY);

    assert_eq!(window.buckets_upto(Period::Day, 10), None);
    assert!(window.buckets_upto(Period::Day, 100).is_some());
}

#[test]
fn a_window_opening_in_the_first_week_a_date_can_represent_has_no_weeks() {
    // Arrange: there is no Monday before the first representable date, so
    // `opening` has nowhere to walk back to.
    let floor = chrono::DateTime::<chrono::Utc>::MIN_UTC.timestamp();
    let window = Window::new(floor, floor + 3 * DAY);

    // Act / Assert: an answer, not a panic.
    assert_eq!(window.weeks(), Vec::new());
    assert_eq!(window.days().len(), 3, "days need no Monday behind them");
}
