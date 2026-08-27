//! Dispute metrics — `docs/SPEC.md` §6.7.
//!
//! A kind 38386 names no order (§2.3), so nothing here pairs a dispute with
//! the trade it is about. What can be counted is what the events say of
//! themselves — status, initiator, when they were opened and when they
//! closed — and one aggregate ratio: disputes opened against orders that
//! found a taker, the population a dispute can arise from.
//!
//! # Which timestamp a metric is counted on
//!
//! Opened-side figures (count, status, initiator, rate) are dated by
//! `opened_at`; outcome and resolution time by the first terminal version,
//! because a dispute resolved in August is August's resolution whenever it
//! was opened. `open_now` and the age of the oldest are about the clock, as
//! in [`crate::activity`], and are left out of monthly blocks for the same
//! reason.

use std::collections::BTreeMap;

use crate::metric::{Metric, Value};
use crate::percentile::percentile;
use crate::window::Window;

/// The five statuses mostrod publishes (`docs/SPEC.md` §2.3). Defined here
/// again because this crate cannot see the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Initiated,
    InProgress,
    SellerRefunded,
    Settled,
    Released,
}

impl Status {
    /// Every status, in the order the reports list them.
    pub const ALL: [Self; 5] = [
        Self::Initiated,
        Self::InProgress,
        Self::SellerRefunded,
        Self::Settled,
        Self::Released,
    ];

    /// The outcomes: the statuses a dispute does not leave.
    pub const TERMINAL: [Self; 3] = [Self::SellerRefunded, Self::Settled, Self::Released];

    pub fn is_terminal(self) -> bool {
        Self::TERMINAL.contains(&self)
    }

    /// The metric-name segment: snake case, so a consumer splitting on the
    /// dot sees one token.
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Initiated => "initiated",
            Self::InProgress => "in_progress",
            Self::SellerRefunded => "seller_refunded",
            Self::Settled => "settled",
            Self::Released => "released",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Initiator {
    Buyer,
    Seller,
}

impl Initiator {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Buyer => "buyer",
            Self::Seller => "seller",
        }
    }
}

/// One dispute as the metrics see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispute {
    pub dispute_id: String,
    /// The instance label, as in [`crate::activity::Order::instance`].
    pub instance: String,
    /// When it was opened: the `created_at` tag, or the first version seen
    /// when no version published one.
    pub opened_at: i64,
    /// From the latest version.
    pub status: Status,
    pub initiator: Option<Initiator>,
    /// `created_at` of the first version in a terminal status.
    pub resolved_at: Option<i64>,
}

/// One order that left `pending` — the population disputes arise from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub order_id: String,
    pub instance: String,
    /// When a taker arrived: the first `in-progress` version, or the
    /// settlement when none was seen.
    pub left_pending_at: i64,
}

/// Everything the §6.7 figures are computed from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisputeData {
    pub disputes: Vec<Dispute>,
    pub taken: Vec<Taken>,
}

/// The ways `stats disputes --by` can slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    /// The count per status — a histogram, not a grouping.
    Status,
    /// The share per initiator — likewise.
    Initiator,
    Instance,
    /// Calendar months inside the window, each reported as its own window.
    Month,
}

/// The §6.7 figures for one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Disputes {
    /// Opened in the window.
    pub opened: u64,
    /// Of those, how many now stand in each status, in [`Status::ALL`] order.
    pub by_status: [u64; 5],
    /// Of those with a known initiator, the buyer's share; `None` when none
    /// is known. The seller's is the complement.
    pub buyer_share: Option<f64>,
    /// `opened / orders that left pending in the window`; `None` when no
    /// order did.
    pub rate: Option<f64>,
    /// Resolved in the window.
    pub resolved: u64,
    /// Of those, the share per outcome, in [`Status::TERMINAL`] order;
    /// `None` when nothing was resolved.
    pub outcome: Option<[f64; 3]>,
    /// `resolved_at − opened_at` over disputes resolved in the window.
    pub resolution_p50: Option<i64>,
    pub resolution_p90: Option<i64>,
    /// Not in a terminal status, whenever opened. About *now*.
    pub open_now: u64,
    /// `now − opened_at` of the oldest open dispute; `None` when none is
    /// open. Also about *now*.
    pub open_oldest_age: Option<i64>,
}

/// The §6.7 figures for `data` over `window`, with `now` dating the open
/// ones.
pub fn summarise(data: &DisputeData, window: Window, now: i64) -> Disputes {
    let opened: Vec<&Dispute> = data
        .disputes
        .iter()
        .filter(|dispute| window.contains(dispute.opened_at))
        .collect();

    let mut by_status = [0; 5];
    for dispute in &opened {
        let index = Status::ALL
            .iter()
            .position(|status| *status == dispute.status)
            .expect("every status is listed");
        by_status[index] += 1;
    }

    let with_initiator = opened.iter().filter(|d| d.initiator.is_some()).count();
    let buyers = opened
        .iter()
        .filter(|d| d.initiator == Some(Initiator::Buyer))
        .count();

    let taken = data
        .taken
        .iter()
        .filter(|order| window.contains(order.left_pending_at))
        .count();

    let resolved: Vec<&Dispute> = data
        .disputes
        .iter()
        .filter(|dispute| dispute.resolved_at.is_some_and(|at| window.contains(at)))
        .collect();
    let outcome = (!resolved.is_empty()).then(|| {
        Status::TERMINAL.map(|status| {
            resolved.iter().filter(|d| d.status == status).count() as f64 / resolved.len() as f64
        })
    });
    let resolution: Vec<i64> = resolved
        .iter()
        .filter_map(|d| d.resolved_at.map(|at| at - d.opened_at))
        .collect();

    let open: Vec<&Dispute> = data
        .disputes
        .iter()
        .filter(|dispute| !dispute.status.is_terminal())
        .collect();

    Disputes {
        opened: opened.len() as u64,
        by_status,
        buyer_share: ratio(buyers, with_initiator),
        rate: ratio(opened.len(), taken),
        resolved: resolved.len() as u64,
        outcome,
        resolution_p50: percentile(&resolution, 0.5),
        resolution_p90: percentile(&resolution, 0.9),
        open_now: open.len() as u64,
        open_oldest_age: open.iter().map(|d| now - d.opened_at).max(),
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

/// `data` split by instance label, keys in sorted order.
pub fn by_instance(data: &DisputeData) -> BTreeMap<String, DisputeData> {
    let mut groups: BTreeMap<String, DisputeData> = BTreeMap::new();

    for dispute in &data.disputes {
        groups
            .entry(dispute.instance.clone())
            .or_default()
            .disputes
            .push(dispute.clone());
    }
    for order in &data.taken {
        groups
            .entry(order.instance.clone())
            .or_default()
            .taken
            .push(order.clone());
    }

    groups
}

/// The report for `stats disputes --by <dimension>`, or the global one when
/// `dimension` is `None`. Names follow [`crate::activity::report`].
pub fn report(
    data: &DisputeData,
    window: Window,
    now: i64,
    dimension: Option<Dimension>,
) -> Vec<Metric> {
    match dimension {
        None => metrics("disputes", &summarise(data, window, now)),
        Some(Dimension::Status) => status_metrics("disputes", &summarise(data, window, now)),
        Some(Dimension::Initiator) => initiator_metrics("disputes", &summarise(data, window, now)),
        Some(Dimension::Month) => window
            .months()
            .into_iter()
            .flat_map(|(key, month)| {
                dated_metrics(&format!("disputes.{key}"), &summarise(data, month, now))
            })
            .collect(),
        Some(Dimension::Instance) => by_instance(data)
            .into_iter()
            .flat_map(|(key, group)| {
                metrics(&format!("disputes.{key}"), &summarise(&group, window, now))
            })
            .collect(),
    }
}

/// One [`Disputes`] as every metric of §6.7, all observed.
pub fn metrics(prefix: &str, disputes: &Disputes) -> Vec<Metric> {
    let mut metrics = dated_metrics(prefix, disputes);
    metrics.extend([
        count(prefix, "open_now", disputes.open_now),
        seconds(prefix, "open_oldest_age", disputes.open_oldest_age),
    ]);
    metrics
}

/// The metrics that are about the window, leaving out the two about *now*.
pub fn dated_metrics(prefix: &str, disputes: &Disputes) -> Vec<Metric> {
    let mut metrics = vec![count(prefix, "opened", disputes.opened)];
    metrics.extend(status_metrics(prefix, disputes));
    metrics.extend(initiator_metrics(prefix, disputes));
    metrics.push(ratio_metric(prefix, "rate", disputes.rate));
    metrics.push(count(prefix, "resolved", disputes.resolved));
    for (index, status) in Status::TERMINAL.iter().enumerate() {
        metrics.push(ratio_metric(
            prefix,
            &format!("outcome.{}", status.as_key()),
            disputes.outcome.map(|shares| shares[index]),
        ));
    }
    metrics.push(seconds(prefix, "resolution_p50", disputes.resolution_p50));
    metrics.push(seconds(prefix, "resolution_p90", disputes.resolution_p90));
    metrics
}

fn status_metrics(prefix: &str, disputes: &Disputes) -> Vec<Metric> {
    Status::ALL
        .iter()
        .zip(disputes.by_status)
        .map(|(status, n)| count(prefix, &format!("status.{}", status.as_key()), n))
        .collect()
}

fn initiator_metrics(prefix: &str, disputes: &Disputes) -> Vec<Metric> {
    vec![
        ratio_metric(prefix, "initiator.buyer", disputes.buyer_share),
        ratio_metric(
            prefix,
            "initiator.seller",
            disputes.buyer_share.map(|buyer| 1.0 - buyer),
        ),
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

fn seconds(prefix: &str, name: &str, value: Option<i64>) -> Metric {
    Metric::observed(
        format!("{prefix}.{name}"),
        value.map_or(Value::Missing, Value::Seconds),
    )
}

#[cfg(test)]
mod tests;
