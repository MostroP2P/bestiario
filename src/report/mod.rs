//! Rendering of computed metrics.
//!
//! Responsibility: the two output formats of `docs/SPEC.md` §10 — a
//! `comfy-table` table by default and the `{generated_at, range, metrics}`
//! JSON envelope under `--json` — plus the observed/inferred marking of §5,
//! applied here once so that no individual metric has to remember it.
//!
//! # One report, two renderings
//!
//! A command builds a [`Report`] — the window it covers and the metrics it
//! computed — and hands it to [`Report::render`]. Nothing in the metrics
//! knows which format it is headed for, so `--json` cannot show a number the
//! table does not, and the `(inf)` mark and the `"kind"` field come from the
//! same [`MetricKind`] and cannot disagree.

use serde::Serialize;

use crate::commands::range::Range;
use crate::stats::{Metric, MetricKind, Value};

pub mod json;
pub mod table;

/// Which format the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
}

impl Format {
    /// `--json` sets it; nothing else does.
    pub fn from_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Table }
    }
}

/// What a command computed: the window it looked at, and what it found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Wall clock at render time, RFC 3339.
    pub generated_at: String,
    pub range: WindowSpan,
    pub metrics: Vec<Metric>,
}

/// The window as the JSON envelope carries it (`docs/SPEC.md` §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowSpan {
    pub from: String,
    pub until: String,
}

impl Report {
    /// A report over `range`, stamped with `now`.
    ///
    /// `now` is a parameter for the reason every clock in this crate is: a
    /// report rendered twice from the same data has to be the same report.
    pub fn new(range: Range, metrics: Vec<Metric>, now: i64) -> Self {
        let (from, until) = range.to_rfc3339();

        Self {
            generated_at: crate::commands::range::format_timestamp(now),
            range: WindowSpan { from, until },
            metrics,
        }
    }

    pub fn render(&self, format: Format) -> String {
        match format {
            Format::Table => table::render(self),
            Format::Json => json::render(self),
        }
    }
}

/// How a metric name reads in a table: `(inf)` after an inferred one.
///
/// The mark is short on purpose. It goes on every inferred row of every
/// table, and a longer one would stop being read.
pub(crate) fn labelled(metric: &Metric) -> String {
    match metric.kind {
        MetricKind::Observed => metric.name.clone(),
        MetricKind::Inferred => format!("{} (inf)", metric.name),
    }
}

/// A value as a person reads it.
///
/// Ratios become percentages and durations become minutes or hours,
/// because the table is for people; the JSON keeps the raw value and the
/// unit for programs.
pub(crate) fn display(value: &Value) -> String {
    match value {
        Value::Count(count) => count.to_string(),
        Value::Sats(sats) => format!("{sats} sats"),
        Value::Ratio(ratio) => format!("{:.1}%", ratio * 100.0),
        Value::Seconds(seconds) => duration(*seconds),
        Value::Fiat { amount, code } => format!("{amount:.2} {code}"),
        Value::Text(text) => text.clone(),
        Value::Missing => "—".to_string(),
    }
}

/// Seconds in the largest unit that still reads as a whole-ish number.
fn duration(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    if seconds < MINUTE {
        format!("{seconds}s")
    } else if seconds < HOUR {
        format!("{:.1}m", seconds as f64 / MINUTE as f64)
    } else if seconds < DAY {
        format!("{:.1}h", seconds as f64 / HOUR as f64)
    } else {
        format!("{:.1}d", seconds as f64 / DAY as f64)
    }
}

#[cfg(test)]
mod tests;
