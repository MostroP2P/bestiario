//! Observed volume — `docs/SPEC.md` §6.2, the rows that need no rate.
//!
//! Everything here is a sum, a percentile or a count over the orders that
//! reached `success` in the window, dated by `success_at` like every
//! completed-side figure in [`crate::activity`]. The two inferred rows of
//! §6.2 — the conversion into a reference currency and the volume implied
//! by dev fees — are roadmap PRs 35 and 36 and live beside this, marked as
//! what they are.
//!
//! Fiat figures are per currency and skip range orders: a range order
//! names no single fiat amount, so it has sats to add and no fiat to add.
//! The sats of a currency are summed over every completed order in it, the
//! range ones included, so the currencies partition the window's sats
//! exactly. The two populations differ, so each carries its own count.

use std::collections::BTreeMap;

use crate::activity::{self, Direction, Order, Status};
use crate::bucket::{self, Coverage};
use crate::metric::{Metric, Value};
use crate::percentile::percentile;
use crate::rates::RateBook;
use crate::window::{Period, Window};

/// The size buckets of §6.2 — `<10k`, `10k–50k`, `50k–200k`, `200k–1M`,
/// `>1M` — as `(label, inclusive upper bound in sats)`; the last one has
/// none. A named boundary belongs to the bucket it names the top of:
/// exactly one million sats is a `200k–1M` order, not a `>1M` one.
pub const BUCKETS: [(&str, Option<i64>); 5] = [
    ("lt_10k", Some(9_999)),
    ("10k_50k", Some(50_000)),
    ("50k_200k", Some(200_000)),
    ("200k_1m", Some(1_000_000)),
    ("gt_1m", None),
];

/// The bucket `size` falls in, as an index into [`BUCKETS`].
pub fn bucket(size: i64) -> usize {
    BUCKETS
        .iter()
        .position(|(_, upper)| upper.is_none_or(|upper| size <= upper))
        .expect("the last bucket is open")
}

/// `∑ sats`, or `None` when the sum leaves `i64` — beyond every satoshi
/// there is, and so a corrupt input rather than a figure.
pub fn sum_sats(sats: impl Iterator<Item = i64>) -> Option<i64> {
    sats.map(i128::from).sum::<i128>().try_into().ok()
}

/// The ways `stats volume --by` can slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Kind,
    Fiat,
    Instance,
    /// Calendar months inside the window, each reported as its own window.
    Month,
    /// Calendar days inside the window, likewise: one block per day, empty
    /// days included, and days the archive predates reported as missing
    /// rather than as zero (see [`crate::bucket`]).
    Day,
}

impl Dimension {
    /// The grouping of orders this dimension asks for, or `None` when it
    /// asks for a period instead — which cuts the window rather than the
    /// orders, and is answered by [`report`] before this is consulted.
    pub fn grouping(self) -> Option<activity::Dimension> {
        match self {
            Self::Kind => Some(activity::Dimension::Kind),
            Self::Fiat => Some(activity::Dimension::Fiat),
            Self::Instance => Some(activity::Dimension::Instance),
            Self::Month | Self::Day => None,
        }
    }

    /// The bucket size this dimension cuts the window into. A dimension
    /// that groups orders instead reports monthly, which is what the
    /// caller asks for only when [`grouping`](Self::grouping) is `None`.
    fn period(self) -> Period {
        match self {
            Self::Day => Period::Day,
            _ => Period::Month,
        }
    }
}

/// One currency's side of the window.
#[derive(Debug, Clone, PartialEq)]
pub struct FiatVolume {
    /// Completed fixed-amount orders in this currency — the population the
    /// fiat figures are computed over, and their denominator.
    pub orders: u64,
    /// Every completed order in this currency, the range ones included.
    /// Never below `orders`, and equal to it when no range order completed.
    pub completed: u64,
    /// `∑ amount_sats` over those; `None` as for [`sum_sats`], and
    /// withheld on its own — the fiat figures are summed from other
    /// numbers and are not touched by a sats overflow.
    pub sats: Option<i64>,
    /// The fiat figures; `None` when the amounts, each finite, add up to
    /// something that is not — then no figure of the fiat side is trusted,
    /// not the tickets either, since they come from the same amounts.
    pub figures: Option<FiatFigures>,
}

/// The finite figures of one currency.
#[derive(Debug, Clone, PartialEq)]
pub struct FiatFigures {
    /// `∑ fiat_amount`.
    pub total: f64,
    pub ticket_avg: f64,
    pub ticket_p50: f64,
    pub ticket_p90: f64,
}

/// The observed §6.2 figures for one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Volume {
    /// Orders that reached `success` in the window.
    pub completed: u64,
    /// `∑ amount_sats` over them; `None` only if the sum leaves `i64`
    /// ([`sum_sats`]).
    pub sats: Option<i64>,
    /// Ticket sizes in sats; `None` when nothing completed. The average
    /// is rounded to the nearest sat, halves up — a sat is indivisible,
    /// and floor would bias every average down.
    pub ticket_avg: Option<i64>,
    pub ticket_p50: Option<i64>,
    pub ticket_p90: Option<i64>,
    /// The largest completed order.
    pub largest: Option<i64>,
    /// Completed orders per size bucket, in [`BUCKETS`] order.
    pub buckets: [u64; 5],
    /// The sats split by the maker's side; `None` as for `sats`.
    pub buy_sats: Option<i64>,
    pub sell_sats: Option<i64>,
    /// Per currency code, over the fixed-amount orders only.
    pub fiat: BTreeMap<String, FiatVolume>,
}

/// `∑ amount_sats` of the orders that reached `success` in `window`;
/// `None` as for [`sum_sats`].
pub fn observed_sats(orders: &[Order], window: Window) -> Option<i64> {
    sum_sats(completed(orders, window).map(|order| order.amount_sats))
}

/// The orders that reached `success` in `window`.
pub fn completed(orders: &[Order], window: Window) -> impl Iterator<Item = &Order> {
    orders
        .iter()
        .filter(|order| order.status == Status::Success)
        .filter(move |order| order.success_at.is_some_and(|at| window.contains(at)))
}

/// The §6.2 observed figures for `orders` over `window`.
pub fn summarise(orders: &[Order], window: Window) -> Volume {
    let done: Vec<&Order> = completed(orders, window).collect();
    let sizes: Vec<i64> = done.iter().map(|order| order.amount_sats).collect();

    let mut buckets = [0; 5];
    for &size in &sizes {
        buckets[bucket(size)] += 1;
    }

    let side = |direction: Direction| {
        sum_sats(
            done.iter()
                .filter(|order| order.direction == direction)
                .map(|order| order.amount_sats),
        )
    };

    // Keyed on the currency and not on the presence of a fiat amount: a
    // range order belongs to its currency's sats even though it belongs to
    // no fiat total.
    let mut by_fiat: BTreeMap<String, (Vec<f64>, Vec<i64>)> = BTreeMap::new();
    for order in &done {
        let (amounts, sats) = by_fiat.entry(order.fiat_code.clone()).or_default();
        if let Some(amount) = order.fiat_amount {
            amounts.push(amount);
        }
        sats.push(order.amount_sats);
    }
    let fiat = by_fiat
        .into_iter()
        .map(|(code, (amounts, sats))| (code, fiat_volume(&amounts, &sats)))
        .collect();

    Volume {
        completed: done.len() as u64,
        sats: sum_sats(sizes.iter().copied()),
        ticket_avg: mean_sats(&sizes),
        ticket_p50: percentile(&sizes, 0.5),
        ticket_p90: percentile(&sizes, 0.9),
        largest: sizes.iter().copied().max(),
        buckets,
        buy_sats: side(Direction::Buy),
        sell_sats: side(Direction::Sell),
        fiat,
    }
}

/// The mean of `sizes` to the nearest sat, halves up; `None` over nothing.
fn mean_sats(sizes: &[i64]) -> Option<i64> {
    if sizes.is_empty() {
        return None;
    }
    let n = sizes.len() as i128;
    let sum: i128 = sizes.iter().map(|&size| i128::from(size)).sum();
    // (sum / n) rounded half up, in integers: (2·sum + n) div (2·n).
    (2 * sum + n).div_euclid(2 * n).try_into().ok()
}

/// One currency's figures: the fiat side over its `amounts`, each finite
/// and each from a fixed-amount order, and the sats side over `sats`, one
/// per completed order in the currency. The fiat side is withheld whole
/// when the amounts add up to something that is not finite; the sats side
/// is withheld on its own when its sum leaves `i64`. Neither withholds the
/// other: they are sums over different numbers.
fn fiat_volume(amounts: &[f64], sats: &[i64]) -> FiatVolume {
    let total: f64 = amounts.iter().sum();
    let figures = (total.is_finite() && !amounts.is_empty()).then(|| FiatFigures {
        total,
        ticket_avg: total / amounts.len() as f64,
        ticket_p50: percentile(amounts, 0.5).expect("finite and non-empty"),
        ticket_p90: percentile(amounts, 0.9).expect("finite and non-empty"),
    });
    FiatVolume {
        orders: amounts.len() as u64,
        completed: sats.len() as u64,
        sats: sum_sats(sats.iter().copied()),
        figures,
    }
}

/// What `stats volume --in <CURRENCY>` asks for: the book to price the
/// orders from, and the currency to price them in.
#[derive(Debug, Clone, Copy)]
pub struct Conversion<'a> {
    pub book: &'a RateBook,
    pub code: &'a str,
}

/// The report for `stats volume --by <dimension>`, or the global one when
/// `dimension` is `None`. Names follow [`crate::activity::report`]. With a
/// `conversion`, the inferred rows of [`converted`] follow the observed
/// ones of every block.
pub fn report(
    orders: &[Order],
    window: Window,
    dimension: Option<Dimension>,
    conversion: Option<Conversion<'_>>,
    coverage: Coverage,
) -> Vec<Metric> {
    let block = |prefix: &str, orders: &[Order], window: Window| {
        let mut block = metrics(prefix, &summarise(orders, window));
        if let Some(Conversion { book, code }) = conversion {
            block.extend(converted::metrics(
                prefix,
                &converted::convert(orders, window, book, code),
            ));
        }
        block
    };

    let Some(dimension) = dimension else {
        return block("volume", orders, window);
    };

    // A dimension either groups the orders or cuts the window; the two are
    // the whole of it, so neither arm can be dead.
    match dimension.grouping() {
        Some(by) => activity::slice(orders, by)
            .into_iter()
            .flat_map(|(key, group)| {
                let group: Vec<Order> = group.into_iter().cloned().collect();
                block(&format!("volume.{key}"), &group, window)
            })
            .collect(),
        None => bucket::walk(window, dimension.period(), coverage, |key, bucket, _| {
            block(&format!("volume.{key}"), orders, bucket)
        }),
    }
}

/// One [`Volume`] as metrics, all observed.
pub fn metrics(prefix: &str, volume: &Volume) -> Vec<Metric> {
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);
    let sats =
        |name: &str, value: Option<i64>| observed(name, value.map_or(Value::Missing, Value::Sats));

    let mut metrics = vec![
        sats("sats", volume.sats),
        observed("completed", Value::Count(volume.completed as i64)),
        sats("ticket_avg", volume.ticket_avg),
        sats("ticket_p50", volume.ticket_p50),
        sats("ticket_p90", volume.ticket_p90),
        sats("largest", volume.largest),
    ];
    for ((label, _), count) in BUCKETS.iter().zip(volume.buckets) {
        metrics.push(observed(
            &format!("size.{label}"),
            Value::Count(count as i64),
        ));
    }
    metrics.push(sats("buy_sats", volume.buy_sats));
    metrics.push(sats("sell_sats", volume.sell_sats));
    for (code, fiat) in &volume.fiat {
        let figure = |pick: fn(&FiatFigures) -> f64| {
            fiat.figures.as_ref().map_or(Value::Missing, |figures| {
                Value::fiat(pick(figures), code.clone())
            })
        };
        metrics.push(observed(
            &format!("fiat.{code}.total"),
            figure(|figures| figures.total),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.orders"),
            Value::Count(fiat.orders as i64),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.sats"),
            fiat.sats.map_or(Value::Missing, Value::Sats),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.completed"),
            Value::Count(fiat.completed as i64),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.ticket_avg"),
            figure(|figures| figures.ticket_avg),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.ticket_p50"),
            figure(|figures| figures.ticket_p50),
        ));
        metrics.push(observed(
            &format!("fiat.{code}.ticket_p90"),
            figure(|figures| figures.ticket_p90),
        ));
    }

    metrics
}

pub mod converted;

#[cfg(test)]
mod tests;
