//! Activity metrics — `docs/SPEC.md` §6.1.
//!
//! Everything here is a function of a list of [`Order`]s, a [`Window`] and a
//! clock. The loader in the binary builds the orders; nothing in this module
//! knows where they came from.
//!
//! # Which timestamp a metric is counted on
//!
//! Each figure is dated by the event that defines it, not by the order's
//! creation: an order created in July and completed in August is one of
//! July's *created* and one of August's *completed*. That is what makes
//! consecutive windows add up, and what makes a completion rate over a
//! window a statement about what happened *in* it rather than about a
//! cohort that started in it.
//!
//! The exception is the abandonment rate, which is a cohort figure by
//! definition — of the orders *created* in the window, how many died without
//! a taker — and is documented as such on [`Activity::abandonment_rate`].

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Timelike, Utc};

use crate::metric::{Metric, Value};
use crate::window::Window;

/// The four statuses that reach the wire (`docs/SPEC.md` §7).
///
/// Defined again here rather than imported from the parser, because this
/// crate cannot see the parser. The loader maps one to the other in a single
/// `match` that the compiler keeps exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    InProgress,
    Success,
    Canceled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in-progress",
            Self::Success => "success",
            Self::Canceled => "canceled",
        }
    }
}

/// Which side the maker is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Buy,
    Sell,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

/// One order as the activity metrics see it: its lifecycle timestamps and
/// the dimensions it can be sliced on.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub order_id: String,
    pub pubkey: String,
    /// How the instance is referred to in a slice — `name (short pubkey)`
    /// when it publishes a name, the bare pubkey otherwise (`docs/SPEC.md`
    /// §3). Chosen by the loader; unique per instance either way.
    pub instance: String,
    /// `created_at` of the first version seen.
    pub created_at: i64,
    pub status: Status,
    pub direction: Direction,
    pub fiat_code: String,
    pub payment_methods: Vec<String>,
    /// From the latest version. Not a §6.1 figure, but what a share of the
    /// network's volume (§6.5) and the summary's sats volume (§6.10) are
    /// summed from; see [`crate::volume`].
    pub amount_sats: i64,
    /// From the latest version; `None` for a range order, which names no
    /// single amount and so contributes nothing to a fiat sum (§6.2).
    pub fiat_amount: Option<f64>,
    /// The premium over the market price, in percent, as published.
    pub premium: f64,
    /// `amt = 0` on the first version: the sats are set at market price
    /// when the order is taken (§4 `price_type`).
    pub is_market_price: bool,
    /// `[min, max]` of the first version, for a range order (§4 `range`).
    pub fiat_range: Option<(f64, f64)>,
    /// `created_at` of the first `in-progress` version — when a taker arrived.
    pub taken_at: Option<i64>,
    /// `created_at` of the first `success` version.
    pub success_at: Option<i64>,
    /// `created_at` of the first `canceled` version.
    pub canceled_at: Option<i64>,
    /// From the latest version; what decides whether a `pending` order is
    /// still open.
    pub expires_at: Option<i64>,
}

/// The ways `stats orders --by` can slice (`docs/SPEC.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Status,
    Kind,
    Fiat,
    Method,
    Instance,
    /// Calendar months inside the window, each reported as its own window.
    Month,
    /// Hour of day, UTC — a histogram rather than a grouping.
    Hour,
    /// Day of week, UTC — a histogram rather than a grouping.
    Weekday,
}

/// The §6.1 figures for one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Activity {
    /// Orders whose first version falls in the window.
    pub created: u64,
    /// Orders that reached `success` in the window.
    pub completed: u64,
    /// Orders that reached `canceled` in the window, expiry included.
    pub canceled: u64,
    /// `completed / (completed + canceled)`; `None` when neither happened.
    pub completion_rate: Option<f64>,
    /// Of the orders created in the window, the share that went straight
    /// from `pending` to `canceled` without a taker; `None` when nothing was
    /// created.
    pub abandonment_rate: Option<f64>,
    /// `pending` orders whose `expires_at` is still ahead of the clock.
    /// A figure about *now*, not about the window.
    pub open_now: u64,
    /// `in-progress` orders, whatever their age. Also about *now*.
    pub in_progress_now: u64,
    /// Growth of `created` against the previous window of the same length;
    /// `None` when that window had none to grow from.
    pub created_delta: Option<f64>,
    /// The same for `completed`.
    pub completed_delta: Option<f64>,
}

/// Counts of created and completed orders per bucket — 24 hours or 7 days.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub labels: Vec<String>,
    pub created: Vec<u64>,
    pub completed: Vec<u64>,
}

/// The §6.1 figures for `orders` over `window`, with `now` deciding what is
/// still open. The deltas compare against the window of the same length
/// before this one.
pub fn summarise(orders: &[Order], window: Window, now: i64) -> Activity {
    summarise_against(orders, window, Some(window.previous()), now)
}

/// [`summarise`], with the deltas compared against `previous` — or reported
/// as missing when there is no window to compare against.
///
/// Split out because a calendar month is not preceded by "the same number of
/// seconds": see [`Window::previous_month`].
pub fn summarise_against(
    orders: &[Order],
    window: Window,
    previous: Option<Window>,
    now: i64,
) -> Activity {
    let current = Counts::over(orders, window);
    let previous = previous.map_or(Counts::NONE, |previous| Counts::over(orders, previous));

    let abandoned = orders
        .iter()
        .filter(|order| window.contains(order.created_at))
        .filter(|order| order.status == Status::Canceled && order.taken_at.is_none())
        .count() as u64;

    Activity {
        created: current.created,
        completed: current.completed,
        canceled: current.canceled,
        completion_rate: ratio(current.completed, current.completed + current.canceled),
        abandonment_rate: ratio(abandoned, current.created),
        open_now: orders
            .iter()
            .filter(|order| order.status == Status::Pending)
            .filter(|order| order.expires_at.is_some_and(|expires| expires > now))
            .count() as u64,
        in_progress_now: orders
            .iter()
            .filter(|order| order.status == Status::InProgress)
            .count() as u64,
        created_delta: growth(current.created, previous.created),
        completed_delta: growth(current.completed, previous.completed),
    }
}

/// The three dated counts of a window, so the current and the previous one
/// are computed by the same code.
struct Counts {
    created: u64,
    completed: u64,
    canceled: u64,
}

impl Counts {
    /// What a window nobody counted holds — the base for a delta with no
    /// previous period, which [`growth`] then reports as missing.
    const NONE: Self = Self {
        created: 0,
        completed: 0,
        canceled: 0,
    };

    fn over(orders: &[Order], window: Window) -> Self {
        let dated = |pick: fn(&Order) -> Option<i64>| {
            orders
                .iter()
                .filter(|order| pick(order).is_some_and(|at| window.contains(at)))
                .count() as u64
        };

        Self {
            created: dated(|order| Some(order.created_at)),
            completed: dated(|order| {
                (order.status == Status::Success)
                    .then_some(order.success_at)
                    .flatten()
            }),
            canceled: dated(|order| {
                (order.status == Status::Canceled)
                    .then_some(order.canceled_at)
                    .flatten()
            }),
        }
    }
}

/// `numerator / denominator`, or nothing when there is nothing to divide by.
///
/// `None` rather than zero, because zero is an answer — "none of them
/// completed" — and this is the absence of one.
fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

/// `(current - previous) / previous`, or nothing when there was no previous.
fn growth(current: u64, previous: u64) -> Option<f64> {
    (previous > 0).then(|| (current as f64 - previous as f64) / previous as f64)
}

/// `orders` grouped by `dimension`, keys in sorted order.
///
/// An order offering several payment methods lands in each of them: a slice
/// by method asks "how much activity involves this method", and an order
/// that involves two involves both.
///
/// Only the grouping dimensions: months are windows, not groups, and the
/// two histograms are neither. Those are handled by [`report`].
pub fn slice(orders: &[Order], dimension: Dimension) -> BTreeMap<String, Vec<&Order>> {
    let mut groups: BTreeMap<String, Vec<&Order>> = BTreeMap::new();

    for order in orders {
        let keys: Vec<String> = match dimension {
            Dimension::Status => vec![order.status.as_str().to_string()],
            Dimension::Kind => vec![order.direction.as_str().to_string()],
            Dimension::Fiat => vec![order.fiat_code.clone()],
            Dimension::Method => order.payment_methods.clone(),
            Dimension::Instance => vec![order.instance.clone()],
            Dimension::Month | Dimension::Hour | Dimension::Weekday => Vec::new(),
        };

        for key in keys {
            groups.entry(key).or_default().push(order);
        }
    }

    groups
}

/// Created and completed orders in the window, by hour of day (UTC).
pub fn by_hour(orders: &[Order], window: Window) -> Histogram {
    histogram(
        orders,
        window,
        24,
        |at| at.hour() as usize,
        |hour| format!("{hour:02}"),
    )
}

/// Created and completed orders in the window, by day of week (UTC),
/// Monday first.
pub fn by_weekday(orders: &[Order], window: Window) -> Histogram {
    const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

    histogram(
        orders,
        window,
        7,
        |at| at.weekday().num_days_from_monday() as usize,
        |day| DAYS[day].to_string(),
    )
}

fn histogram(
    orders: &[Order],
    window: Window,
    buckets: usize,
    bucket_of: fn(DateTime<Utc>) -> usize,
    label_of: impl Fn(usize) -> String,
) -> Histogram {
    let mut created = vec![0; buckets];
    let mut completed = vec![0; buckets];

    let bucket = |timestamp: i64| DateTime::<Utc>::from_timestamp(timestamp, 0).map(bucket_of);

    for order in orders {
        if window.contains(order.created_at)
            && let Some(bucket) = bucket(order.created_at)
        {
            created[bucket] += 1;
        }
        if order.status == Status::Success
            && let Some(at) = order.success_at.filter(|at| window.contains(*at))
            && let Some(bucket) = bucket(at)
        {
            completed[bucket] += 1;
        }
    }

    Histogram {
        labels: (0..buckets).map(label_of).collect(),
        created,
        completed,
    }
}

/// The report for `stats orders --by <dimension>`, or the global one when
/// `dimension` is `None`.
///
/// Metric names are dotted paths, with the slice as a segment:
/// `orders.created` globally, `orders.ARS.created` sliced by fiat,
/// `orders.hour.14.created` for the histogram. The same name means the same
/// thing in every report, and a JSON consumer can split on the dot.
pub fn report(
    orders: &[Order],
    window: Window,
    now: i64,
    dimension: Option<Dimension>,
) -> Vec<Metric> {
    match dimension {
        None => metrics("orders", &summarise(orders, window, now)),
        Some(Dimension::Month) => window
            .months()
            .into_iter()
            .flat_map(|(key, month)| {
                let activity = summarise_against(orders, month, month.previous_month(), now);
                dated_metrics(&format!("orders.{key}"), &activity)
            })
            .collect(),
        Some(Dimension::Hour) => histogram_metrics("orders.hour", &by_hour(orders, window)),
        Some(Dimension::Weekday) => {
            histogram_metrics("orders.weekday", &by_weekday(orders, window))
        }
        Some(dimension) => slice(orders, dimension)
            .into_iter()
            .flat_map(|(key, group)| {
                let group: Vec<Order> = group.into_iter().cloned().collect();
                metrics(&format!("orders.{key}"), &summarise(&group, window, now))
            })
            .collect(),
    }
}

/// One [`Activity`] as the nine metrics of §6.1, all observed.
pub fn metrics(prefix: &str, activity: &Activity) -> Vec<Metric> {
    let mut metrics = dated_metrics(prefix, activity);
    metrics.extend([
        count(prefix, "open_now", activity.open_now),
        count(prefix, "in_progress_now", activity.in_progress_now),
    ]);
    metrics
}

/// The seven metrics of §6.1 that are about the window, leaving out the two
/// that are about *now*.
///
/// A monthly report is a sequence of windows, and `open_now` is the same
/// number in every one of them: it is a statement about the clock, not
/// about July. Repeating it under each month would make the blocks read as
/// if they added up when they do not, so a month gets only what it counted.
pub fn dated_metrics(prefix: &str, activity: &Activity) -> Vec<Metric> {
    vec![
        count(prefix, "created", activity.created),
        count(prefix, "completed", activity.completed),
        count(prefix, "canceled", activity.canceled),
        ratio_metric(prefix, "completion_rate", activity.completion_rate),
        ratio_metric(prefix, "abandonment_rate", activity.abandonment_rate),
        ratio_metric(prefix, "created_delta", activity.created_delta),
        ratio_metric(prefix, "completed_delta", activity.completed_delta),
    ]
}

fn count(prefix: &str, name: &str, value: u64) -> Metric {
    Metric::observed(format!("{prefix}.{name}"), Value::Count(value as i64))
}

fn ratio_metric(prefix: &str, name: &str, value: Option<f64>) -> Metric {
    Metric::observed(
        format!("{prefix}.{name}"),
        value.map_or(Value::Missing, Value::ratio),
    )
}

fn histogram_metrics(prefix: &str, histogram: &Histogram) -> Vec<Metric> {
    histogram
        .labels
        .iter()
        .zip(histogram.created.iter().zip(&histogram.completed))
        .flat_map(|(label, (created, completed))| {
            [
                Metric::observed(
                    format!("{prefix}.{label}.created"),
                    Value::Count(*created as i64),
                ),
                Metric::observed(
                    format!("{prefix}.{label}.completed"),
                    Value::Count(*completed as i64),
                ),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests;
