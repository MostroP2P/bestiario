use super::*;
use crate::metric::Value;

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
