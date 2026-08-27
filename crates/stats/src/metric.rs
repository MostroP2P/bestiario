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

use serde::{Serialize, Serializer};

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
///
/// # Floating-point values
///
/// `f64` admits `NaN` and the infinities, and neither is a figure: JSON
/// cannot carry them, and a table cannot read them. The two variants that
/// hold one are therefore treated as [`Missing`](Self::Missing) wherever
/// they are rendered — see [`normalised`](Self::normalised) — and the
/// constructors [`ratio`](Self::ratio) and [`fiat`](Self::fiat) make that
/// substitution up front, so an aggregation dividing by zero produces a
/// missing figure rather than a `NaN` that is one only at the output.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A plain count. Signed: a delta against the previous period is one.
    Count(i64),
    Sats(i64),
    /// A finite fraction, rendered as a percentage. Usually in `0..=1`; a
    /// growth figure against the previous period can be negative or exceed
    /// one, so the range is not enforced — finiteness is.
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

impl Value {
    /// A ratio, or [`Missing`](Self::Missing) when `ratio` is not a finite
    /// number.
    pub fn ratio(ratio: f64) -> Self {
        if ratio.is_finite() {
            Self::Ratio(ratio)
        } else {
            Self::Missing
        }
    }

    /// A fiat amount, or [`Missing`](Self::Missing) when `amount` is not a
    /// finite number.
    pub fn fiat(amount: f64, code: impl Into<String>) -> Self {
        if amount.is_finite() {
            Self::Fiat {
                amount,
                code: code.into(),
            }
        } else {
            Self::Missing
        }
    }

    /// The value as a renderer should see it: itself, unless it holds a
    /// non-finite number, in which case [`Missing`](Self::Missing).
    ///
    /// The variants are public, so a `Value::Ratio(f64::NAN)` can be built;
    /// this is the one place that decides what it means, and both output
    /// formats go through it.
    pub fn normalised(&self) -> &Self {
        match self {
            Self::Ratio(ratio) if !ratio.is_finite() => &Self::Missing,
            Self::Fiat { amount, .. } if !amount.is_finite() => &Self::Missing,
            other => other,
        }
    }
}

/// The wire shape of a [`Value`]: `{"unit": …, "value": …}`.
///
/// A separate enum rather than a derive on `Value` itself for two reasons
/// the derive cannot express: `Missing` has to carry `"value": null` so
/// that every metric record has a `value` member (`docs/SPEC.md` §10), and
/// a non-finite number has to become that same shape rather than
/// `serde_json`'s silent `null` under `"unit": "ratio"`.
#[derive(Serialize)]
#[serde(tag = "unit", content = "value", rename_all = "lowercase")]
enum Wire<'a> {
    Count(i64),
    Sats(i64),
    Ratio(f64),
    Seconds(i64),
    Fiat { amount: f64, code: &'a str },
    Text(&'a str),
    Missing(()),
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match self.normalised() {
            Value::Count(count) => Wire::Count(*count),
            Value::Sats(sats) => Wire::Sats(*sats),
            Value::Ratio(ratio) => Wire::Ratio(*ratio),
            Value::Seconds(seconds) => Wire::Seconds(*seconds),
            Value::Fiat { amount, code } => Wire::Fiat {
                amount: *amount,
                code,
            },
            Value::Text(text) => Wire::Text(text),
            Value::Missing => Wire::Missing(()),
        };

        wire.serialize(serializer)
    }
}

/// One named figure, of one kind, with the error that qualifies it.
///
/// `kind` and `error` are private and move together: the only way to get an
/// inferred metric is [`Metric::inferred`], which demands the error, and the
/// only way to get an observed one is [`Metric::observed`], which admits
/// none. `name` and `value` carry no invariant and stay public.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metric {
    pub name: String,
    kind: MetricKind,
    #[serde(flatten)]
    pub value: Value,
    /// What makes an inferred figure uncertain. Always `None` for an observed
    /// one — there is nothing between the events and the number.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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

    /// The same figure with no value: what a report says about a period
    /// the archive cannot speak for. The name, the kind and the error
    /// survive, since they describe the figure and not the answer.
    pub fn missing(self) -> Self {
        Self {
            value: Value::Missing,
            ..self
        }
    }

    pub fn kind(&self) -> MetricKind {
        self.kind
    }

    pub fn is_inferred(&self) -> bool {
        self.kind == MetricKind::Inferred
    }

    /// The error of an inferred figure; `None` for an observed one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests;
