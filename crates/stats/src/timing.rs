//! Timing and the funnel — `docs/SPEC.md` §6.4 and §7: how long each
//! stage of an order takes, how old the open book is, and how many
//! published orders find a taker. `stats timing [--by fiat|method|kind|instance]`.
//!
//! Every duration is the gap between two *observed* versions of the same
//! order — the payoff for persisting every version (§4) — and every figure
//! is observed. Nothing here is anchored on the first version bestiario
//! happened to catch: kind 38383 is replaceable, a relay keeps only the
//! latest state, and a backfill therefore meets most orders mid-flight or
//! at their end. An order whose `pending` version was never seen has no
//! book entry to measure from; it is counted as of *unknown origin* and
//! left out of every figure that starts at the book.
//!
//! # One lifecycle per order
//!
//! The projection keeps the first `success` and the first `canceled`
//! version independently, and a malformed history can carry both. The
//! *canonical terminal* is the earlier of the two; an order with both is
//! counted under `regressed`, and only its canonical end is measured.
//!
//! # Which orders a figure is counted on
//!
//! Each duration is dated by the version that ends it, the rule of
//! [`crate::activity`]: fills by `taken_at`, completions and full cycles
//! by the success, cancellations by the cancellation. Each percentile
//! reports its own sample count, since the populations differ: a fill
//! needs `pending` and `in-progress`, a completion `in-progress` and
//! `success`, a full cycle `pending` and `success`.
//!
//! The funnel is over the orders whose `pending` version was seen in the
//! window. A success implies a taker — the protocol has no other path to
//! it — so a completed order counts as taken even when its `in-progress`
//! version was missed. A cancellation with no `in-progress` version seen
//! is *canceled untaken*: within a cohort observed from its book entry,
//! that is the evidence there is. A `pending` order past its expiry with
//! no terminal version seen is *expired untaken*, not open.
//!
//! The book is about *now*: the `pending` orders seen from their book
//! entry, not taken, not ended, published no later than now and expiring
//! after it. Slices are by the dimensions of the first version seen
//! ([`Order::origin`]), so a later republication cannot move an already
//! counted entry to another slice.

use std::collections::BTreeMap;

use crate::activity::Order;
use crate::metric::{Metric, Value};
use crate::percentile::percentile;
use crate::window::Window;

/// The ways `stats timing --by` can slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Fiat,
    Method,
    Kind,
    Instance,
}

/// How an order's observed history ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    Success(i64),
    Canceled(i64),
}

/// The canonical end of `order`: the earlier of its first `success` and
/// first `canceled` versions.
fn terminal(order: &Order) -> Option<Terminal> {
    match (order.success_at, order.canceled_at) {
        (Some(success), Some(canceled)) if canceled < success => Some(Terminal::Canceled(canceled)),
        (Some(success), _) => Some(Terminal::Success(success)),
        (None, Some(canceled)) => Some(Terminal::Canceled(canceled)),
        (None, None) => None,
    }
}

/// The age of `order` on the book at `now` — seen from its `pending`
/// version, not taken, not ended, published no later than now and
/// expiring after it — or `None` when it is not on the book, or when its
/// clock is so far off that its age is not a duration.
fn book_age(order: &Order, now: i64) -> Option<i64> {
    let on_book = order.taken_at.is_none()
        && terminal(order).is_none()
        && order.expires_at.is_some_and(|expires| expires > now);
    if !on_book {
        return None;
    }
    let pending = order.pending_at.filter(|pending| *pending <= now)?;
    now.checked_sub(pending)
}

/// What became of the orders that entered the book in a window (§7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Funnel {
    /// Orders whose `pending` version was seen in the window.
    pub created: u64,
    /// Found a taker: an `in-progress` version was seen, or a success was.
    pub taken: u64,
    pub completed: u64,
    /// Canceled after an `in-progress` version was seen.
    pub canceled_taken: u64,
    /// Canceled with no `in-progress` version seen.
    pub canceled_untaken: u64,
    /// Past its expiry with no terminal version seen, and never taken.
    pub expired_untaken: u64,
    /// No terminal version seen and not expired: on the book, or taken
    /// and in progress.
    pub open: u64,
}

impl Funnel {
    fn share(&self, count: u64) -> Option<f64> {
        (self.created > 0).then(|| count as f64 / self.created as f64)
    }

    /// `taken / created`: the fill rate.
    pub fn taken_share(&self) -> Option<f64> {
        self.share(self.taken)
    }

    /// `canceled_untaken / created`.
    pub fn canceled_untaken_share(&self) -> Option<f64> {
        self.share(self.canceled_untaken)
    }
}

/// The §6.4 figures for one window. Durations in seconds, nearest-rank
/// percentiles, `None` over nothing; each with the count of samples it
/// is taken over.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timing {
    /// `taken_at − pending_at`, over orders taken in the window.
    pub time_to_fill_samples: u64,
    pub time_to_fill_p50: Option<i64>,
    pub time_to_fill_p90: Option<i64>,
    /// `success − taken_at`, over orders completed in the window.
    pub time_to_complete_samples: u64,
    pub time_to_complete_p50: Option<i64>,
    pub time_to_complete_p90: Option<i64>,
    /// `success − pending_at`, over orders completed in the window.
    pub full_cycle_samples: u64,
    pub full_cycle_p50: Option<i64>,
    pub full_cycle_p90: Option<i64>,
    /// `canceled − pending_at`, over orders canceled in the window.
    pub time_to_cancel_samples: u64,
    pub time_to_cancel_p50: Option<i64>,
    pub time_to_cancel_p90: Option<i64>,
    /// Orders on the book now.
    pub book_size: u64,
    /// Their mean age, `now − pending_at`, to the second.
    pub book_age_avg: Option<i64>,
    pub funnel: Funnel,
    /// Orders first seen in the window at a version later than `pending`:
    /// their book entry was never observed, so they are in no cohort.
    pub unknown_origin: u64,
    /// Orders with both a success and a cancellation seen; only the
    /// earlier one is counted.
    pub regressed: u64,
}

/// The §6.4 and §7 figures for `orders` over `window`, with `now` deciding
/// what is live.
pub fn summarise(orders: &[Order], window: Window, now: i64) -> Timing {
    // A gap from `from` to `to`, counted when `to` falls in the window and
    // is strictly later; a zero or negative gap is a malformed history,
    // not a duration, and one that overflows is not a number.
    let gaps = |from: fn(&Order) -> Option<i64>, to: fn(&Order) -> Option<i64>| -> Vec<i64> {
        orders
            .iter()
            .filter_map(|order| from(order).zip(to(order)))
            .filter(|(_, to)| window.contains(*to))
            .filter_map(|(from, to)| to.checked_sub(from))
            .filter(|gap| *gap > 0)
            .collect()
    };
    let success = |order: &Order| match terminal(order) {
        Some(Terminal::Success(at)) => Some(at),
        _ => None,
    };
    let canceled = |order: &Order| match terminal(order) {
        Some(Terminal::Canceled(at)) => Some(at),
        _ => None,
    };
    let fills = gaps(|o| o.pending_at, |o| o.taken_at);
    let completes = gaps(|o| o.taken_at, success);
    let cycles = gaps(|o| o.pending_at, success);
    let cancels = gaps(|o| o.pending_at, canceled);

    let ages: Vec<i128> = orders
        .iter()
        .filter_map(|order| book_age(order, now))
        .map(i128::from)
        .collect();
    let book_age_avg = (!ages.is_empty())
        .then(|| ages.iter().sum::<i128>() / ages.len() as i128)
        .and_then(|age| age.try_into().ok());

    let mut funnel = Funnel::default();
    for order in orders.iter().filter(|order| {
        order
            .pending_at
            .is_some_and(|pending| window.contains(pending))
    }) {
        funnel.created += 1;
        let end = terminal(order);
        let taken = order.taken_at.is_some() || matches!(end, Some(Terminal::Success(_)));
        if taken {
            funnel.taken += 1;
        }
        match end {
            Some(Terminal::Success(_)) => funnel.completed += 1,
            Some(Terminal::Canceled(_)) if order.taken_at.is_some() => funnel.canceled_taken += 1,
            Some(Terminal::Canceled(_)) => funnel.canceled_untaken += 1,
            None if !taken && order.expires_at.is_some_and(|expires| expires <= now) => {
                funnel.expired_untaken += 1;
            }
            None => funnel.open += 1,
        }
    }

    Timing {
        time_to_fill_samples: fills.len() as u64,
        time_to_fill_p50: percentile(&fills, 0.5),
        time_to_fill_p90: percentile(&fills, 0.9),
        time_to_complete_samples: completes.len() as u64,
        time_to_complete_p50: percentile(&completes, 0.5),
        time_to_complete_p90: percentile(&completes, 0.9),
        full_cycle_samples: cycles.len() as u64,
        full_cycle_p50: percentile(&cycles, 0.5),
        full_cycle_p90: percentile(&cycles, 0.9),
        time_to_cancel_samples: cancels.len() as u64,
        time_to_cancel_p50: percentile(&cancels, 0.5),
        time_to_cancel_p90: percentile(&cancels, 0.9),
        book_size: ages.len() as u64,
        book_age_avg,
        funnel,
        unknown_origin: orders
            .iter()
            .filter(|order| order.pending_at.is_none() && window.contains(order.created_at))
            .count() as u64,
        regressed: orders
            .iter()
            .filter(|order| order.success_at.is_some() && order.canceled_at.is_some())
            .count() as u64,
    }
}

/// `orders` grouped by the dimension of their first version seen — or by
/// instance, which does not change — keys in sorted order.
pub fn slice(orders: &[Order], dimension: Dimension) -> BTreeMap<String, Vec<&Order>> {
    let mut groups: BTreeMap<String, Vec<&Order>> = BTreeMap::new();
    for order in orders {
        let keys: Vec<String> = match dimension {
            Dimension::Fiat => vec![order.origin.fiat_code.clone()],
            Dimension::Method => order.origin.payment_methods.clone(),
            Dimension::Kind => vec![order.origin.direction.as_str().to_string()],
            Dimension::Instance => vec![order.instance.clone()],
        };
        for key in keys {
            groups.entry(key).or_default().push(order);
        }
    }
    groups
}

/// The report for `stats timing --by <dimension>`, or the global one when
/// `dimension` is `None`. Names follow [`crate::activity::report`].
pub fn report(
    orders: &[Order],
    window: Window,
    dimension: Option<Dimension>,
    now: i64,
) -> Vec<Metric> {
    let Some(dimension) = dimension else {
        return metrics("timing", &summarise(orders, window, now));
    };

    slice(orders, dimension)
        .into_iter()
        .flat_map(|(key, group)| {
            let group: Vec<Order> = group.into_iter().cloned().collect();
            metrics(&format!("timing.{key}"), &summarise(&group, window, now))
        })
        .collect()
}

/// One [`Timing`] as metrics, all observed.
pub fn metrics(prefix: &str, timing: &Timing) -> Vec<Metric> {
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);
    let count = |name: &str, value: u64| observed(name, Value::Count(value as i64));
    let seconds = |name: &str, value: Option<i64>| {
        observed(name, value.map_or(Value::Missing, Value::Seconds))
    };
    let ratio =
        |name: &str, value: Option<f64>| observed(name, value.map_or(Value::Missing, Value::ratio));
    let funnel = &timing.funnel;

    vec![
        count("time_to_fill_samples", timing.time_to_fill_samples),
        seconds("time_to_fill_p50", timing.time_to_fill_p50),
        seconds("time_to_fill_p90", timing.time_to_fill_p90),
        count("time_to_complete_samples", timing.time_to_complete_samples),
        seconds("time_to_complete_p50", timing.time_to_complete_p50),
        seconds("time_to_complete_p90", timing.time_to_complete_p90),
        count("full_cycle_samples", timing.full_cycle_samples),
        seconds("full_cycle_p50", timing.full_cycle_p50),
        seconds("full_cycle_p90", timing.full_cycle_p90),
        count("time_to_cancel_samples", timing.time_to_cancel_samples),
        seconds("time_to_cancel_p50", timing.time_to_cancel_p50),
        seconds("time_to_cancel_p90", timing.time_to_cancel_p90),
        count("book_size", timing.book_size),
        seconds("book_age_avg", timing.book_age_avg),
        count("funnel.created", funnel.created),
        count("funnel.taken", funnel.taken),
        ratio("funnel.taken_share", funnel.taken_share()),
        count("funnel.completed", funnel.completed),
        count("funnel.canceled_taken", funnel.canceled_taken),
        count("funnel.canceled_untaken", funnel.canceled_untaken),
        ratio(
            "funnel.canceled_untaken_share",
            funnel.canceled_untaken_share(),
        ),
        count("funnel.expired_untaken", funnel.expired_untaken),
        count("funnel.open", funnel.open),
        count("unknown_origin", timing.unknown_origin),
        count("regressed", timing.regressed),
    ]
}

#[cfg(test)]
mod tests;
