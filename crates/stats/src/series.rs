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
use crate::rates::RateBook;
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
    /// What to price the orders in, for the converted §6.2 rows. `None`
    /// leaves them out, exactly as `stats volume` does without `--in`.
    pub priced: Option<Priced>,
}

/// The rate snapshots to price orders from, and the currency to price them
/// in — what `volume.in.<CODE>.…` is computed from.
///
/// Owned rather than borrowed like [`crate::volume::Conversion`], because
/// [`Data`] is what a caller hands over and holding a borrow would make the
/// book the caller's problem to keep alive across the whole series.
#[derive(Debug, Clone, PartialEq)]
pub struct Priced {
    pub book: RateBook,
    pub code: String,
}

/// The currency a `volume.in.<CODE>.…` metric asks to be priced in.
///
/// The name carries it, so nothing else has to be told: a series of a
/// converted figure knows which book to load from the metric it was asked
/// for. `None` for every other metric, and for a code that is not one — the
/// three uppercase letters an instance publishes.
pub fn priced_in(metric: &str) -> Option<String> {
    let code = metric
        .strip_prefix(&format!("{}.in.", Family::Volume.prefix()))?
        .split('.')
        .next()?;

    let canonical = code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_uppercase());
    canonical.then(|| code.to_string())
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
                let mut metrics =
                    volume::metrics(self.prefix(), &volume::summarise(&data.orders, window));
                // §6.2's converted rows, on the same terms as `stats volume
                // --in`: present when there is a book and a currency to
                // price in, and inferred when they are.
                if let Some(priced) = &data.priced {
                    metrics.extend(volume::converted::metrics(
                        self.prefix(),
                        &volume::converted::convert(
                            &data.orders,
                            window,
                            &priced.book,
                            &priced.code,
                        ),
                    ));
                }
                metrics
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
        "that range holds more than the {MAX_BUCKETS} buckets a series prints; \
         ask for a wider `--by` or a shorter range"
    )]
    TooManyBuckets,
}

/// How many buckets one series may have.
///
/// A day-by-day series over an archive that reaches back years is not a
/// table anybody reads, and computing it costs every family over every day.
/// The limit is high enough for a decade of months, three years of weeks or
/// two years of days.
pub const MAX_BUCKETS: usize = 800;

/// Every metric that can be plotted against `data` over `window`, in family
/// order.
///
/// Derived from what the families actually report over it, so it grows
/// with them — and shrinks when an assumption this run does not have would
/// be needed.
///
/// Over `window` rather than over a fixed instant because half of these
/// names are the data's, not the code's: `volume.fiat.ARS.total` exists
/// because some order completed in ARS, and which currencies those are is a
/// question only a real window can answer. A catalogue taken over the epoch
/// would list the metrics every archive has and none of the ones this
/// archive is actually about.
pub fn catalogue(data: &Data, window: Window, now: i64) -> Vec<String> {
    [
        Family::Activity,
        Family::Volume,
        Family::DevFees,
        Family::Disputes,
    ]
    .into_iter()
    .flat_map(|family| family.metrics(data, window, now))
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

/// `metric` over `window` as its family reports it — value *and*
/// provenance — or `None` when no family reports a metric by that name.
///
/// The whole metric rather than its number, because a series has to say
/// what a report says: `dev_fees.implied_volume` rests on an assumed fee
/// share, and a bucket of it that came back as a bare figure would be
/// printed without its `(inf)` marker and serialised as an observation.
pub fn measure(data: &Data, window: Window, now: i64, metric: &str) -> Option<Metric> {
    Family::of(metric)?
        .metrics(data, window, now)
        .into_iter()
        .find(|candidate| candidate.name == metric)
}

/// The value of `metric` over `window`, or `None` when no family reports
/// a metric by that name.
pub fn value(data: &Data, window: Window, now: i64, metric: &str) -> Option<Value> {
    measure(data, window, now, metric).map(|candidate| candidate.value)
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

/// `value` under `name`, inferred when any of `sources` is — carrying the
/// first qualification they give.
///
/// A figure derived from an estimate is an estimate: §5's `(inf)` marker is
/// about where a number came from, and renaming one into a bucket does not
/// turn an assumption into an observation.
fn qualified_as<'a>(
    name: String,
    value: Value,
    sources: impl IntoIterator<Item = Option<&'a Metric>>,
) -> Metric {
    let qualification = sources
        .into_iter()
        .flatten()
        .filter(|source| source.is_inferred())
        .find_map(|source| source.error());

    match qualification {
        Some(error) => Metric::inferred(name, value, error),
        None => Metric::observed(name, value),
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
                            // The book prices every slice, not the whole
                            // alone: dropping it here would answer a split
                            // series of a converted metric with nothing.
                            priced: data.priced.clone(),
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
    // Asked of the window being plotted, because a name like
    // `volume.fiat.ARS.total` exists only where an ARS order completed: over
    // a fixed instant every per-currency and per-instance metric this
    // archive has would be turned away as unknown.
    if value(data, window, now, metric).is_none() {
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

    // Capped as it is built, not counted afterwards: the range is as wide
    // as the caller typed, and building a hundred trillion buckets to find
    // out there are too many is the thing the limit is for.
    let buckets = window
        .buckets_upto(period, MAX_BUCKETS)
        .ok_or(SeriesError::TooManyBuckets)?;

    // A bucket is the source metric with a new name, provenance included:
    // an inferred figure stays inferred, carrying what qualifies it, and so
    // does a Δ taken between two of them — a change between two estimates is
    // an estimate. A bucket the family does not report at all is missing,
    // and a missing figure is nobody's inference.
    let plot = |prefix: &str, data: &Data| {
        let mut metrics = Vec::new();
        let mut previous: Option<Metric> = None;
        for (key, bucket) in &buckets {
            let found = measure(data, *bucket, now, metric);
            let current = found
                .as_ref()
                .map_or(Value::Missing, |metric| metric.value.clone());
            let change = previous
                .as_ref()
                .map_or(Value::Missing, |previous| delta(&previous.value, &current));

            metrics.push(qualified_as(
                format!("{prefix}.{key}"),
                current,
                [found.as_ref()],
            ));
            metrics.push(qualified_as(
                format!("{prefix}.{key}.delta"),
                change,
                [found.as_ref(), previous.as_ref()],
            ));
            previous = found;
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
