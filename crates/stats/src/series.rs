//! One metric over time — `bestiario series <METRIC> --by <PERIOD>`
//! (`docs/SPEC.md` §6.10 view 4).
//!
//! The command asks for a metric by name and gets it once per bucket, with
//! the change against the bucket before it. Nothing here knows the metrics
//! individually: a bucket is evaluated by asking each family for the whole
//! block it reports over that window and looking the name up in it. A
//! metric added to [`crate::activity`], [`crate::volume`],
//! [`crate::dev_fees`] or [`crate::disputes`] is therefore series-able the
//! day it lands, without a line changing here — which is the indirection
//! the roadmap asks for.
//!
//! # What cannot be a series
//!
//! Two kinds of figure are refused rather than plotted. Those about *now*
//! — the open book, the disputes still open — are the same number in every
//! bucket, since they are not functions of the window at all. And those
//! that are already a change against a previous period: a Δ of a Δ answers
//! nothing. Both are named in the error, with the metrics that do work.

use std::collections::BTreeMap;

use crate::activity::{self, Order};
use crate::dev_fees::{self, DevFeeData};
use crate::disputes::{self, DisputeData};
use crate::metric::{Metric, Value};
use crate::volume;
use crate::window::{Period, Window};

/// Everything the four families are computed from.
#[derive(Debug, Clone, Default)]
pub struct Data {
    pub orders: Vec<Order>,
    pub fees: DevFeeData,
    pub disputes: DisputeData,
    /// The dev fee share to assume per instance (§5). `None` leaves the
    /// inferred §6.6 rows out: they rest on an assumption, and a series
    /// asked for without one would plot a number nothing supports.
    pub dev_fee_pct: Option<Assumption>,
}

/// The `dev_fee_percentage` assumption, as `dev_fees::implied` needs it:
/// the operator's per-instance overrides and the default for the rest.
///
/// Carried as data rather than as the closure the aggregation takes, so
/// that this crate stays free of the configuration it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Assumption {
    pub per_instance: BTreeMap<String, f64>,
    pub default: f64,
}

impl Assumption {
    fn share(&self, pubkey: &str) -> f64 {
        self.per_instance
            .get(pubkey)
            .copied()
            .unwrap_or(self.default)
    }
}

/// The metric families a series can be taken over (§6.1, §6.2, §6.6, §6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Activity,
    Volume,
    DevFees,
    Disputes,
}

impl Family {
    /// The prefix every one of its metrics carries.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Activity => "orders",
            Self::Volume => "volume",
            Self::DevFees => "dev_fees",
            Self::Disputes => "disputes",
        }
    }

    /// The family a metric name belongs to.
    pub fn of(metric: &str) -> Option<Self> {
        [Self::Activity, Self::Volume, Self::DevFees, Self::Disputes]
            .into_iter()
            .find(|family| metric.starts_with(&format!("{}.", family.prefix())))
    }

    /// The whole block this family reports over `window`.
    fn metrics(self, data: &Data, window: Window, now: i64) -> Vec<Metric> {
        match self {
            Self::Activity => activity::metrics(
                self.prefix(),
                &activity::summarise(&data.orders, window, now),
            ),
            Self::Volume => {
                volume::metrics(self.prefix(), &volume::summarise(&data.orders, window))
            }
            Self::DevFees => {
                let mut metrics =
                    dev_fees::metrics(self.prefix(), &dev_fees::summarise(&data.fees, window));
                if let Some(assumption) = &data.dev_fee_pct {
                    let share = |pubkey: &str| assumption.share(pubkey);
                    metrics.extend(dev_fees::implied::metrics(
                        self.prefix(),
                        &dev_fees::implied::summarise(&data.fees, window, &share),
                    ));
                }
                metrics
            }
            Self::Disputes => disputes::metrics(
                self.prefix(),
                &disputes::summarise(&data.disputes, window, now),
            ),
        }
    }

    /// Whether this family can be broken down by `split`.
    fn supports(self, split: Split) -> bool {
        match split {
            Split::Instance => true,
            // A dev fee names no currency and a dispute names neither a
            // currency nor a side: the event carries no such tag (§2.3,
            // §2.4), and a block of zeros per currency would read as an
            // answer.
            Split::Kind | Split::Fiat => matches!(self, Self::Activity | Self::Volume),
        }
    }
}

/// The ways a bucket can be broken down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    Instance,
    Kind,
    Fiat,
}

impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Kind => "kind",
            Self::Fiat => "fiat",
        }
    }
}

/// Why a series could not be plotted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SeriesError {
    #[error("`{metric}` is not a metric that can be plotted over time")]
    UnknownMetric { metric: String },

    #[error(
        "`{metric}` is a figure about now, not about a period, so it would be \
         the same number in every bucket"
    )]
    AboutNow { metric: String },

    #[error("`{metric}` is already a change against a previous period")]
    AlreadyADelta { metric: String },

    #[error("`{metric}` cannot be split by {}: the events it counts carry no such tag", split.as_str())]
    CannotSplit { metric: String, split: Split },

    #[error(
        "{buckets} buckets is more than the {MAX_BUCKETS} a series prints; \
         ask for a wider `--by` or a shorter range"
    )]
    TooManyBuckets { buckets: usize },
}

/// How many buckets one series may have.
///
/// A day-by-day series over an archive that reaches back years is not a
/// table anybody reads, and computing it costs every family over every day.
/// The limit is high enough for a decade of months, three years of weeks or
/// two years of days.
pub const MAX_BUCKETS: usize = 800;

/// Every metric that can be plotted against `data`, in family order.
///
/// Derived from what the families actually report over it, so it grows
/// with them — and shrinks when an assumption this run does not have would
/// be needed.
pub fn catalogue(data: &Data, now: i64) -> Vec<String> {
    [
        Family::Activity,
        Family::Volume,
        Family::DevFees,
        Family::Disputes,
    ]
    .into_iter()
    .flat_map(|family| family.metrics(data, Window::new(0, 1), now))
    .map(|metric| metric.name)
    .filter(|name| plottable(name).is_ok())
    .collect()
}

/// Whether `name` is a figure of its window, and so has a shape over time.
fn plottable(name: &str) -> Result<(), SeriesError> {
    let metric = name.to_string();
    if name.ends_with("_now") {
        return Err(SeriesError::AboutNow { metric });
    }
    // `disputes.open.1.id` and friends: the open book itself, listed.
    if name.contains(".open.") {
        return Err(SeriesError::AboutNow { metric });
    }
    if name.ends_with("_delta") {
        return Err(SeriesError::AlreadyADelta { metric });
    }
    Ok(())
}

/// The value of `metric` over `window`, or `None` when no family reports
/// a metric by that name.
pub fn value(data: &Data, window: Window, now: i64, metric: &str) -> Option<Value> {
    Family::of(metric)?
        .metrics(data, window, now)
        .into_iter()
        .find(|candidate| candidate.name == metric)
        .map(|candidate| candidate.value)
}

/// The change from `previous` to `current`.
///
/// A relative change for a magnitude — a count, a sum, a duration, an
/// amount — since that is what "grew by a third" means. For a figure that
/// is already a proportion, the arithmetic difference instead: a completion
/// rate going from 20% to 30% rose by ten points, not by half. `None`
/// against nothing, and against a previous zero, which no proportion of
/// anything reaches.
pub fn delta(previous: &Value, current: &Value) -> Value {
    let (Some(before), Some(after)) = (magnitude(previous), magnitude(current)) else {
        return Value::Missing;
    };

    match (previous, current) {
        (Value::Ratio(_), Value::Ratio(_)) => Value::ratio(after - before),
        _ if before == 0.0 => Value::Missing,
        _ => Value::ratio((after - before) / before.abs()),
    }
}

/// The number inside a value, when it has one.
fn magnitude(value: &Value) -> Option<f64> {
    match value {
        Value::Count(count) => Some(*count as f64),
        Value::Sats(sats) => Some(*sats as f64),
        Value::Seconds(seconds) => Some(*seconds as f64),
        Value::Ratio(ratio) => Some(*ratio),
        Value::Fiat { amount, .. } => Some(*amount),
        Value::Text(_) | Value::Missing => None,
    }
}

/// `data` split by `split`, keys in sorted order — only what `family`
/// needs, since a series of one metric reads one family.
fn split_data(data: &Data, family: Family, split: Split) -> BTreeMap<String, Data> {
    match family {
        Family::Activity | Family::Volume => {
            let dimension = match split {
                Split::Instance => activity::Dimension::Instance,
                Split::Kind => activity::Dimension::Kind,
                Split::Fiat => activity::Dimension::Fiat,
            };
            activity::slice(&data.orders, dimension)
                .into_iter()
                .map(|(key, orders)| {
                    let orders = orders.into_iter().cloned().collect();
                    (
                        key,
                        Data {
                            orders,
                            ..Data::default()
                        },
                    )
                })
                .collect()
        }
        Family::DevFees => dev_fees::by_instance(&data.fees)
            .into_iter()
            .map(|(key, fees)| {
                (
                    key,
                    Data {
                        fees,
                        dev_fee_pct: data.dev_fee_pct.clone(),
                        ..Data::default()
                    },
                )
            })
            .collect(),
        Family::Disputes => disputes::by_instance(&data.disputes)
            .into_iter()
            .map(|(key, disputes)| {
                (
                    key,
                    Data {
                        disputes,
                        ..Data::default()
                    },
                )
            })
            .collect(),
    }
}

/// `metric` over the buckets of `window`, optionally once per slice.
///
/// Names are `<metric>.<bucket>` and `<metric>.<bucket>.delta`, or
/// `<metric>.<key>.<bucket>` with a split — the key where every other
/// report puts it.
pub fn report(
    data: &Data,
    window: Window,
    period: Period,
    metric: &str,
    split: Option<Split>,
    now: i64,
) -> Result<Vec<Metric>, SeriesError> {
    let family = Family::of(metric).ok_or_else(|| SeriesError::UnknownMetric {
        metric: metric.to_string(),
    })?;
    plottable(metric)?;
    // A name with the right prefix that no family reports is still unknown.
    if value(data, Window::new(0, 1), now, metric).is_none() {
        return Err(SeriesError::UnknownMetric {
            metric: metric.to_string(),
        });
    }
    if let Some(split) = split
        && !family.supports(split)
    {
        return Err(SeriesError::CannotSplit {
            metric: metric.to_string(),
            split,
        });
    }

    let buckets = window.buckets(period);
    if buckets.len() > MAX_BUCKETS {
        return Err(SeriesError::TooManyBuckets {
            buckets: buckets.len(),
        });
    }

    let plot = |prefix: &str, data: &Data| {
        let mut metrics = Vec::new();
        let mut previous: Option<Value> = None;
        for (key, bucket) in &buckets {
            let current = value(data, *bucket, now, metric).unwrap_or(Value::Missing);
            metrics.push(Metric::observed(format!("{prefix}.{key}"), current.clone()));
            metrics.push(Metric::observed(
                format!("{prefix}.{key}.delta"),
                previous
                    .as_ref()
                    .map_or(Value::Missing, |previous| delta(previous, &current)),
            ));
            previous = Some(current);
        }
        metrics
    };

    Ok(match split {
        None => plot(metric, data),
        Some(split) => split_data(data, family, split)
            .into_iter()
            .flat_map(|(key, sliced)| plot(&format!("{metric}.{key}"), &sliced))
            .collect(),
    })
}

#[cfg(test)]
mod tests;
