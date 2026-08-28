use super::*;
use crate::metric::Value;
use crate::publish::address::{Bucket, Resolution};

/// 2026-08-27T00:00:00Z, a Thursday.
const THURSDAY: i64 = 1_787_788_800;
const DAY: i64 = 86_400;

/// A block that reports the bucket it was given, so a test can see which
/// window each row was computed over.
fn spy(key: &str, window: Window, previous: Option<Window>) -> Vec<Metric> {
    vec![
        Metric::observed(format!("x.{key}.from"), Value::Count(window.from)),
        Metric::observed(
            format!("x.{key}.previous"),
            previous.map_or(Value::Missing, |window| Value::Count(window.from)),
        ),
        Metric::inferred(format!("x.{key}.guess"), Value::Count(1), "an assumption"),
    ]
}

#[test]
fn a_window_the_archive_reaches_into_is_covered() {
    // Arrange
    let coverage = Coverage::since(THURSDAY);

    // Assert: the boundary belongs to the archive's side.
    assert!(!coverage.covers(Window::new(THURSDAY - DAY, THURSDAY)));
    assert!(coverage.covers(Window::new(THURSDAY - DAY, THURSDAY + 1)));
    assert!(coverage.covers(Window::new(THURSDAY, THURSDAY + DAY)));
}

#[test]
fn an_archive_holding_nothing_can_speak_for_nothing() {
    let coverage = Coverage::from_earliest(None);

    assert!(!coverage.covers(Window::new(0, i64::MAX)));
}

#[test]
fn every_bucket_of_the_window_is_reported() {
    let window = Window::new(THURSDAY, THURSDAY + 3 * DAY);

    let metrics = walk(window, Period::Day, Coverage::since(THURSDAY), spy);

    let names: Vec<&str> = metrics
        .iter()
        .map(|metric| metric.name.as_str())
        .filter(|name| name.ends_with(".from"))
        .collect();
    assert_eq!(
        names,
        [
            "x.2026-08-27.from",
            "x.2026-08-28.from",
            "x.2026-08-29.from"
        ]
    );
}

#[test]
fn a_bucket_before_the_archive_keeps_its_rows_and_withholds_its_values() {
    // The archive begins on the second day of a three-day window.
    let window = Window::new(THURSDAY, THURSDAY + 3 * DAY);

    let metrics = walk(window, Period::Day, Coverage::since(THURSDAY + DAY), spy);

    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == name)
            .expect("present")
            .value
    };
    assert_eq!(value("x.2026-08-27.from"), &Value::Missing, "not zero");
    assert_eq!(
        value("x.2026-08-28.from"),
        &Value::Count(THURSDAY + DAY),
        "the day the archive begins is answerable"
    );
    assert_eq!(
        value("x.2026-08-29.from"),
        &Value::Count(THURSDAY + 2 * DAY)
    );
}

#[test]
fn a_withheld_bucket_keeps_the_kind_and_the_error_of_its_figures() {
    let window = Window::new(THURSDAY, THURSDAY + DAY);

    let metrics = walk(window, Period::Day, Coverage::since(THURSDAY + DAY), spy);
    let guess = metrics
        .iter()
        .find(|metric| metric.name == "x.2026-08-27.guess")
        .expect("present");

    assert_eq!(guess.value, Value::Missing);
    assert!(guess.is_inferred(), "an inferred figure stays inferred");
    assert_eq!(guess.error(), Some("an assumption"));
}

#[test]
fn each_bucket_is_handed_the_one_before_it() {
    let window = Window::new(THURSDAY, THURSDAY + 2 * DAY);

    let metrics = walk(window, Period::Day, Coverage::since(0), spy);

    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == name)
            .expect("present")
            .value
    };
    assert_eq!(
        value("x.2026-08-27.previous"),
        &Value::Count(THURSDAY - DAY),
        "the day before, even though the window does not reach it"
    );
    assert_eq!(value("x.2026-08-28.previous"), &Value::Count(THURSDAY));
}

#[test]
fn a_bucket_whose_predecessor_the_archive_never_saw_is_compared_against_nothing() {
    let window = Window::new(THURSDAY, THURSDAY + 2 * DAY);

    let metrics = walk(window, Period::Day, Coverage::since(THURSDAY), spy);

    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == name)
            .expect("present")
            .value
    };
    assert_eq!(value("x.2026-08-27.previous"), &Value::Missing);
    assert_eq!(value("x.2026-08-28.previous"), &Value::Count(THURSDAY));
}

// ---- the partitions a publication run computes
// (`docs/NOSTR-PUBLICATION.md` §6.3)

/// 2025-12-15T00:00:00Z and 2026-01-10T00:00:00Z.
const DECEMBER: i64 = 1_765_756_800;
const JANUARY: i64 = 1_768_003_200;

#[test]
fn an_archive_holding_nothing_has_no_partition_to_publish() {
    let empty = Coverage::default();

    for resolution in Resolution::ALL {
        assert!(
            empty.partitions(resolution, THURSDAY).is_empty(),
            "{resolution:?}: nothing to speak for is nothing to publish"
        );
    }
}

#[test]
fn an_archive_that_begins_after_now_has_no_partition_either() {
    let ahead = Coverage::since(THURSDAY + DAY);

    assert!(ahead.partitions(Resolution::Monthly, THURSDAY).is_empty());
}

#[test]
fn a_timestamp_no_calendar_date_can_hold_yields_no_partitions() {
    let unrepresentable = Coverage::since(i64::MAX);

    assert!(
        unrepresentable
            .partitions(Resolution::Daily, i64::MAX)
            .is_empty(),
        "a clock past every representable date names no month"
    );
}

#[test]
fn the_month_partitions_roll_over_into_the_next_year() {
    let across = Coverage::since(DECEMBER);

    let months = across.partitions(Resolution::Daily, JANUARY);

    assert_eq!(
        months,
        vec![
            Bucket::Month {
                year: 2025,
                month: 12
            },
            Bucket::Month {
                year: 2026,
                month: 1
            },
        ],
        "December is followed by the January of the next year, not by a thirteenth month"
    );
}

#[test]
fn a_monthly_partition_is_a_year_and_it_spans_every_year_covered() {
    let across = Coverage::since(DECEMBER);

    let years = across.partitions(Resolution::Monthly, JANUARY);

    assert_eq!(years, vec![Bucket::Year(2025), Bucket::Year(2026)]);
}
