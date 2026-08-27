//! Both renderings of one mixed report, so that the `(inf)` mark and the
//! `"kind"` field are checked against the same input.

use super::*;
use crate::stats::Value;

const NOW: i64 = 1_787_800_000;
const FROM: i64 = 1_785_000_000;
const UNTIL: i64 = 1_787_000_000;

fn range() -> Range {
    Range::resolve(Some(FROM), Some(UNTIL), NOW).expect("window")
}

/// One observed count, one inferred figure with an error, one missing value.
fn mixed() -> Report {
    Report::new(
        range(),
        vec![
            Metric::observed("orders.created", Value::Count(42)),
            Metric::inferred(
                "volume.usd",
                Value::Fiat {
                    amount: 1_234.5,
                    code: "USD".into(),
                },
                "rate up to 5 min old",
            ),
            Metric::observed("orders.completion_rate", Value::Missing),
        ],
        NOW,
    )
}

#[test]
fn the_json_envelope_has_the_three_fields_the_spec_names() {
    // Arrange / Act
    let json: serde_json::Value =
        serde_json::from_str(&mixed().render(Format::Json)).expect("valid json");

    // Assert
    assert_eq!(json["generated_at"], "2026-08-27T03:06:40+00:00");
    assert_eq!(json["range"]["from"], "2026-07-25T17:20:00+00:00");
    assert_eq!(json["range"]["until"], "2026-08-17T20:53:20+00:00");
    assert_eq!(json["metrics"].as_array().map(Vec::len), Some(3));
}

#[test]
fn the_json_marks_each_metric_with_its_kind_and_carries_the_error_only_when_there_is_one() {
    let json: serde_json::Value =
        serde_json::from_str(&mixed().render(Format::Json)).expect("valid json");
    let metrics = json["metrics"].as_array().expect("array");

    assert_eq!(metrics[0]["kind"], "observed");
    assert_eq!(metrics[0].get("error"), None);
    assert_eq!(metrics[1]["kind"], "inferred");
    assert_eq!(metrics[1]["error"], "rate up to 5 min old");
    assert_eq!(metrics[1]["value"]["code"], "USD");
    assert_eq!(metrics[2]["unit"], "missing");
    assert_eq!(metrics[2]["value"], serde_json::Value::Null);
}

/// The table line naming `metric`, so a test can check what sits beside it.
fn row<'a>(table: &'a str, metric: &str) -> &'a str {
    table
        .lines()
        .find(|line| line.contains(metric))
        .unwrap_or_else(|| panic!("`{metric}` has a row:\n{table}"))
}

#[test]
fn the_table_puts_each_formatted_value_beside_its_metric() {
    let table = mixed().render(Format::Table);

    assert!(row(&table, "orders.created").contains("42"), "{table}");
    assert!(row(&table, "volume.usd").contains("1234.50 USD"), "{table}");
    assert!(
        row(&table, "volume.usd").contains("rate up to 5 min old"),
        "{table}"
    );
    assert!(
        row(&table, "orders.completion_rate").contains("—"),
        "{table}"
    );
}

#[test]
fn a_non_finite_number_renders_as_missing_in_both_formats() {
    let report = Report::new(
        range(),
        vec![
            Metric::observed("nan", Value::Ratio(f64::NAN)),
            Metric::observed(
                "inf",
                Value::Fiat {
                    amount: f64::INFINITY,
                    code: "USD".into(),
                },
            ),
        ],
        NOW,
    );

    let table = report.render(Format::Table);
    assert!(row(&table, "nan").contains("—"), "{table}");
    assert!(row(&table, "inf").contains("—"), "{table}");
    assert!(!table.contains("NaN"), "{table}");

    let json: serde_json::Value =
        serde_json::from_str(&report.render(Format::Json)).expect("valid json");
    assert_eq!(json["metrics"][0]["unit"], "missing");
    assert_eq!(json["metrics"][0]["value"], serde_json::Value::Null);
    assert_eq!(json["metrics"][1]["unit"], "missing");
}

#[test]
fn the_table_marks_inferred_rows_and_leaves_observed_ones_bare() {
    let table = mixed().render(Format::Table);

    assert!(table.contains("volume.usd (inf)"), "{table}");
    assert!(table.contains("orders.created"), "{table}");
    assert!(!table.contains("orders.created (inf)"), "{table}");
}

#[test]
fn the_table_shows_the_error_column_only_when_a_metric_has_one() {
    let with = mixed().render(Format::Table);
    let without = Report::new(
        range(),
        vec![Metric::observed("orders.created", Value::Count(1))],
        NOW,
    )
    .render(Format::Table);

    assert!(with.contains("error"), "{with}");
    assert!(with.contains("rate up to 5 min old"), "{with}");
    assert!(!without.contains("error"), "{without}");
}

#[test]
fn the_table_reads_values_the_way_a_person_would() {
    assert_eq!(display(&Value::Count(42)), "42");
    assert_eq!(display(&Value::Sats(150_000)), "150000 sats");
    assert_eq!(display(&Value::Ratio(0.375)), "37.5%");
    assert_eq!(display(&Value::Seconds(45)), "45s");
    assert_eq!(display(&Value::Seconds(150)), "2.5m");
    assert_eq!(display(&Value::Seconds(5_400)), "1.5h");
    assert_eq!(display(&Value::Seconds(129_600)), "1.5d");
    assert_eq!(
        display(&Value::Fiat {
            amount: 1_234.5,
            code: "ARS".into()
        }),
        "1234.50 ARS"
    );
    assert_eq!(display(&Value::Missing), "—");
}

#[test]
fn a_ratio_displays_at_the_edges_and_beyond_them() {
    assert_eq!(display(&Value::Ratio(0.0)), "0.0%");
    assert_eq!(display(&Value::Ratio(1.0)), "100.0%");
    // A delta: legitimate, and read as growth.
    assert_eq!(display(&Value::Ratio(2.0)), "200.0%");
    assert_eq!(display(&Value::Ratio(-0.5)), "-50.0%");
    assert_eq!(display(&Value::Ratio(f64::NAN)), "—");
    assert_eq!(display(&Value::Ratio(f64::INFINITY)), "—");
}

#[test]
fn the_table_names_the_window_it_covers() {
    let table = mixed().render(Format::Table);

    assert!(
        table.starts_with("2026-07-25T17:20:00+00:00 — 2026-08-17T20:53:20+00:00"),
        "{table}"
    );
}

#[test]
fn the_format_comes_from_the_flag_alone() {
    assert_eq!(Format::from_flag(true), Format::Json);
    assert_eq!(Format::from_flag(false), Format::Table);
}

/// Two instances, three columns, one metric outside the grid.
fn grid() -> Report {
    Report::new(
        range(),
        vec![
            Metric::observed("compare.Alpha.completed", Value::Count(3)),
            Metric::observed("compare.Alpha.volume", Value::Sats(500)),
            Metric::inferred("compare.Alpha.usd", Value::Count(7), "rate 5m old"),
            Metric::observed("compare.Beta.completed", Value::Count(1)),
            Metric::observed("compare.Beta.volume", Value::Missing),
            Metric::observed("summary.created", Value::Count(99)),
        ],
        NOW,
    )
}

#[test]
fn the_pivoted_table_has_one_row_per_key_and_one_column_per_figure() {
    let table = grid().render_rows(
        Format::Table,
        "instance",
        "compare",
        &["completed", "volume", "usd"],
    );

    let alpha = table
        .lines()
        .find(|line| line.contains("Alpha"))
        .expect("row");
    assert!(alpha.contains("3"), "{table}");
    assert!(alpha.contains("500 sats"), "{table}");
    assert!(alpha.contains("7 (inf)"), "{table}");
    let beta = table
        .lines()
        .find(|line| line.contains("Beta"))
        .expect("row");
    assert!(beta.contains("—"), "{table}");
    assert!(!table.contains("99"), "outside the grid: {table}");
    assert_eq!(
        table.lines().filter(|line| line.contains("│")).count(),
        3,
        "{table}"
    );
}

#[test]
fn the_pivoted_rendering_leaves_the_json_untouched() {
    let pivoted = grid().render_rows(Format::Json, "instance", "compare", &["completed"]);

    assert_eq!(pivoted, grid().render(Format::Json));
}
