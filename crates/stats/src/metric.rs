//! The unit of a report: one named figure, and how much it can be trusted.
//!
//! `docs/SPEC.md` §5 splits every number this tool produces into two kinds.
//! An **observed** figure was published on Nostr and counted. An **inferred**
//! one was derived from something else — a rate that may be minutes old, a
//! fee percentage nobody publishes — and carries the reason it might be
//! wrong.
//!
//! The distinction is carried in the type rather than in the renderer,
//! because a renderer that decides it can only decide from the name, and a
//! metric that grows an inference later would keep the old marking until
//! somebody noticed. Here it cannot be forgotten: an inferred metric is
//! constructed through [`Metric::inferred`], which requires the error.

use serde::Serialize;

/// Whether a figure was measured or estimated (`docs/SPEC.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    Observed,
    Inferred,
}

/// A figure and what it counts.
///
/// The unit travels with the number rather than being spelled into the
/// metric's name, so that a renderer can format a ratio as a percentage and a
/// duration as minutes without parsing English.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "unit", content = "value", rename_all = "lowercase")]
pub enum Value {
    /// A plain count. Signed: a delta against the previous period is one.
    Count(i64),
    Sats(i64),
    /// A fraction in `0..=1`, rendered as a percentage.
    Ratio(f64),
    Seconds(i64),
    Fiat {
        amount: f64,
        code: String,
    },
    Text(String),
    /// Nothing to report.
    ///
    /// Distinct from zero, and the distinction is the point: a completion
    /// rate over a window with no orders in it is not 0%, it is a question
    /// with no answer. Reporting zero there would be a claim the data does
    /// not make.
    Missing,
}

/// One named figure, of one kind, with the error that qualifies it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metric {
    pub name: String,
    pub kind: MetricKind,
    #[serde(flatten)]
    pub value: Value,
    /// What makes an inferred figure uncertain. Always `None` for an observed
    /// one — there is nothing between the events and the number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Metric {
    /// A figure counted straight from what instances published.
    pub fn observed(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            kind: MetricKind::Observed,
            value,
            error: None,
        }
    }

    /// A figure derived from something else, with the reason it may be wrong.
    ///
    /// The error is required rather than optional: §5 says every inferred
    /// figure is reported with its error column, and an inference whose error
    /// nobody could be bothered to write down is one nobody should act on.
    pub fn inferred(name: impl Into<String>, value: Value, error: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: MetricKind::Inferred,
            value,
            error: Some(error.into()),
        }
    }

    pub fn is_inferred(&self) -> bool {
        self.kind == MetricKind::Inferred
    }
}

#[cfg(test)]
mod tests;
