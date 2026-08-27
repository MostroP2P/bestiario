//! The default rendering: one row per metric, `(inf)` marking on the name;
//! and the pivoted one, for views that are a grid.

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};

use super::{Report, display, labelled};
use crate::stats::MetricKind;

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

/// The report as a grid: one row per key between `prefix.` and a column
/// name, one column per entry of `columns`, in the order given.
///
/// A metric that does not fit the grid — outside the prefix, or under a
/// column not listed — is left out of the table rather than forced into a
/// cell; the JSON still carries it. An inferred cell is marked `(inf)`
/// after its value, since the name column is now the row key; the error
/// text has no column here and is where the JSON has it.
pub fn render_rows(report: &Report, row_label: &str, prefix: &str, columns: &[&str]) -> String {
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();

    for metric in &report.metrics {
        let Some(rest) = metric
            .name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('.'))
        else {
            continue;
        };
        let Some((key, column)) = columns.iter().enumerate().find_map(|(index, column)| {
            rest.strip_suffix(column)
                .and_then(|key| key.strip_suffix('.'))
                .map(|key| (key, index))
        }) else {
            continue;
        };

        let position = match rows.iter().position(|(row, _)| row == key) {
            Some(position) => position,
            None => {
                rows.push((key.to_string(), vec![String::new(); columns.len()]));
                rows.len() - 1
            }
        };
        let mut cell = display(&metric.value);
        if metric.kind() == MetricKind::Inferred {
            cell.push_str(" (inf)");
        }
        rows[position].1[column] = cell;
    }

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    let mut header = vec![row_label];
    header.extend(columns);
    table.set_header(header);
    for (key, cells) in rows {
        let mut row = vec![key];
        row.extend(cells);
        table.add_row(row);
    }

    format!("{} — {}\n{table}\n", report.range.from, report.range.until)
}
