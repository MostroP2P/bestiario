//! Timing and the funnel — `docs/SPEC.md` §6.4 and §7: how long each
//! stage of an order takes, how old the open book is, and how many
//! published orders find a taker. `stats timing [--by fiat|method|kind|instance]`.
//!
//! Every duration is the gap between two published versions of the same
//! order — the payoff for persisting every version (§4) — and every figure
//! is observed. An order whose first seen version is already `in-progress`
//! or terminal has no earlier stage to measure from and yields no gap,
//! rather than a zero: a backfill catches many orders mid-flight. The instants come pre-folded in [`Order`]: `created_at`
//! is the first version, `taken_at` the first `in-progress`, `success_at`
//! and `canceled_at` the terminal ones.
//!
//! # Which orders a figure is counted on
//!
//! Each duration is dated by the event that ends it, the rule of
//! [`crate::activity`]: fills by `taken_at`, completions and full cycles
//! by `success_at`, cancellations by `canceled_at`, so consecutive months
//! add up. The funnel is over the orders *created* in the window, since it
//! asks what became of them. The book is about *now*, not the window: the
//! live `pending` orders and their mean age.

use crate::activity::{self, Order, Status};
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

/// What became of the orders created in a window (§7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Funnel {
    pub created: u64,
    /// Found a taker, whatever happened next.
    pub taken: u64,
    /// Canceled or expired without one.
    pub canceled_untaken: u64,
    /// Canceled after one.
    pub canceled_taken: u64,
    pub completed: u64,
    /// No terminal version yet.
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

    /// `canceled_untaken / created`: the orders nobody took.
    pub fn canceled_untaken_share(&self) -> Option<f64> {
        self.share(self.canceled_untaken)
    }
}

/// The §6.4 figures for one window. Durations in seconds, nearest-rank
/// percentiles, `None` over nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timing {
    /// Orders taken in the window.
    pub filled: u64,
    pub time_to_fill_p50: Option<i64>,
    pub time_to_fill_p90: Option<i64>,
    /// Orders completed in the window.
    pub completed: u64,
    pub time_to_complete_p50: Option<i64>,
    pub time_to_complete_p90: Option<i64>,
    pub full_cycle_p50: Option<i64>,
    pub full_cycle_p90: Option<i64>,
    /// Orders canceled in the window.
    pub canceled: u64,
    pub time_to_cancel_p50: Option<i64>,
    pub time_to_cancel_p90: Option<i64>,
    /// Live `pending` orders now.
    pub book_size: u64,
    /// Their mean age, `now − created_at`.
    pub book_age_avg: Option<i64>,
    pub funnel: Funnel,
}

/// The §6.4 and §7 figures for `orders` over `window`, with `now` deciding
/// what is live.
pub fn summarise(orders: &[Order], window: Window, now: i64) -> Timing {
    // A gap from `from` to `to`, counted when `to` falls in the window and
    // `to` is strictly later. A zero gap is an order whose earlier stage
    // was never seen — the first version bestiario caught was already the
    // later one, so both instants are the same event — and a negative one
    // is a malformed history; neither is a duration.
    let gaps = |from: fn(&Order) -> Option<i64>, to: fn(&Order) -> Option<i64>| -> Vec<i64> {
        orders
            .iter()
            .filter_map(|order| from(order).zip(to(order)))
            .filter(|(_, to)| window.contains(*to))
            .filter(|(from, to)| to > from)
            .map(|(from, to)| to - from)
            .collect()
    };
    let fills = gaps(|o| Some(o.created_at), |o| o.taken_at);
    let completes = gaps(|o| o.taken_at, |o| o.success_at);
    let cycles = gaps(|o| Some(o.created_at), |o| o.success_at);
    let cancels = gaps(|o| Some(o.created_at), |o| o.canceled_at);

    let ages: Vec<i64> = orders
        .iter()
        .filter(|order| order.status == Status::Pending)
        .filter(|order| order.expires_at.is_some_and(|expires| expires > now))
        .map(|order| now - order.created_at)
        .collect();

    let mut funnel = Funnel::default();
    for order in orders
        .iter()
        .filter(|order| window.contains(order.created_at))
    {
        funnel.created += 1;
        if order.taken_at.is_some() {
            funnel.taken += 1;
        }
        match order.status {
            Status::Success => funnel.completed += 1,
            Status::Canceled if order.taken_at.is_some() => funnel.canceled_taken += 1,
            Status::Canceled => funnel.canceled_untaken += 1,
            Status::Pending | Status::InProgress => funnel.open += 1,
        }
    }

    Timing {
        filled: fills.len() as u64,
        time_to_fill_p50: percentile(&fills, 0.5),
        time_to_fill_p90: percentile(&fills, 0.9),
        completed: cycles.len() as u64,
        time_to_complete_p50: percentile(&completes, 0.5),
        time_to_complete_p90: percentile(&completes, 0.9),
        full_cycle_p50: percentile(&cycles, 0.5),
        full_cycle_p90: percentile(&cycles, 0.9),
        canceled: cancels.len() as u64,
        time_to_cancel_p50: percentile(&cancels, 0.5),
        time_to_cancel_p90: percentile(&cancels, 0.9),
        book_size: ages.len() as u64,
        book_age_avg: (!ages.is_empty()).then(|| ages.iter().sum::<i64>() / ages.len() as i64),
        funnel,
    }
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

    let by = match dimension {
        Dimension::Fiat => activity::Dimension::Fiat,
        Dimension::Method => activity::Dimension::Method,
        Dimension::Kind => activity::Dimension::Kind,
        Dimension::Instance => activity::Dimension::Instance,
    };
    activity::slice(orders, by)
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
        count("filled", timing.filled),
        seconds("time_to_fill_p50", timing.time_to_fill_p50),
        seconds("time_to_fill_p90", timing.time_to_fill_p90),
        count("completed", timing.completed),
        seconds("time_to_complete_p50", timing.time_to_complete_p50),
        seconds("time_to_complete_p90", timing.time_to_complete_p90),
        seconds("full_cycle_p50", timing.full_cycle_p50),
        seconds("full_cycle_p90", timing.full_cycle_p90),
        count("canceled", timing.canceled),
        seconds("time_to_cancel_p50", timing.time_to_cancel_p50),
        seconds("time_to_cancel_p90", timing.time_to_cancel_p90),
        count("book_size", timing.book_size),
        seconds("book_age_avg", timing.book_age_avg),
        count("funnel.created", funnel.created),
        count("funnel.taken", funnel.taken),
        ratio("funnel.taken_share", funnel.taken_share()),
        count("funnel.canceled_untaken", funnel.canceled_untaken),
        ratio(
            "funnel.canceled_untaken_share",
            funnel.canceled_untaken_share(),
        ),
        count("funnel.canceled_taken", funnel.canceled_taken),
        count("funnel.completed", funnel.completed),
        count("funnel.open", funnel.open),
    ]
}

#[cfg(test)]
mod tests;
