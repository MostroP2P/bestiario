//! The `{generated_at, range, metrics}` envelope of `docs/SPEC.md` §10.
//!
//! Nothing here decides what a metric looks like: [`Report`] and [`Metric`]
//! already derive `Serialize`, and the envelope is the struct itself. This
//! module exists so the pretty-printing and the one failure mode live in one
//! place.

use super::Report;

/// The report as pretty-printed JSON.
///
/// Serialising a `Report` cannot fail — every field is a string, a number or
/// an enum of those — so the `expect` is a statement, not a hope. If a future
/// field makes it fallible, this is the one line that changes.
pub fn render(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("a Report serialises: it holds only plain data")
}
