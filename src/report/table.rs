//! The default rendering: one row per metric, `(inf)` marking on the name.

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};

use super::{Report, display, labelled};

/// The report as a table.
///
/// Three columns, the third present only when some metric has something to
/// put in it: an all-observed report has no errors to show, and an empty
/// column on every row would say "there might have been something here" to
/// a reader who has no way to know there was not.
pub fn render(report: &Report) -> String {
    let has_errors = report.metrics.iter().any(|metric| metric.error().is_some());

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let mut header = vec!["metric", "value"];
    if has_errors {
        header.push("error");
    }
    table.set_header(header);

    for metric in &report.metrics {
        let mut row = vec![labelled(metric), display(&metric.value)];
        if has_errors {
            row.push(metric.error().unwrap_or_default().to_string());
        }
        table.add_row(row);
    }

    format!("{} — {}\n{table}\n", report.range.from, report.range.until)
}
