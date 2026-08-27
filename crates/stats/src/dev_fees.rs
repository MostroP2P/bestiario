//! Dev fee metrics — `docs/SPEC.md` §6.6; the inferred-volume row and its
//! comparison against the observed one are in [`implied`].
//!
//! Two inputs, because the figures look at the fees from both ends. The
//! [`Fee`]s answer how much was sent, how late, how often twice, and how
//! often for an order nobody has seen. The [`Settlement`]s — completed orders
//! — answer the coverage question from the other side: of the trades that
//! should have produced a fee, how many did.
//!
//! # Which timestamp a metric is counted on
//!
//! Fee figures are dated by the fee event; coverage by the order's
//! `success_at`, because it is a property of the trade, not of the payment
//! that may or may not follow it. Both are dated by what defines them, so
//! consecutive months add up — the same rule as [`crate::activity`].

use std::collections::BTreeMap;
use std::collections::HashSet;

use crate::metric::{Metric, Value};
use crate::percentile::percentile;
use crate::window::Window;

/// One kind 8383 event, with what is known about the order it names.
#[derive(Debug, Clone, PartialEq)]
pub struct Fee {
    pub event_id: String,
    pub order_id: String,
    pub pubkey: String,
    /// The instance label, as in [`crate::activity::Order::instance`].
    pub instance: String,
    pub created_at: i64,
    pub amount_sats: i64,
    /// `true` when an earlier fee exists for the same order (mostrod #620);
    /// only the earliest one counts as money sent.
    pub is_duplicate: bool,
    /// Whether the order this fee names has been seen at all. A fee for an
    /// unseen order is an *orphan* — usual during backfill, since fees
    /// outlive orders on the relays by roughly a year to a fortnight.
    pub order_known: bool,
    /// `success_at` of the order, when it is known and completed; what the
    /// payment latency is measured from.
    pub settled_at: Option<i64>,
    /// The instance's `fee` in force when the order settled — or, for an
    /// orphan, when the fee was paid. What [`implied`] divides by; `None`
    /// when no kind 38385 in force said.
    pub fee_in_force: Option<f64>,
    /// `amount_sats` of the order, when it is known **and** reached
    /// `success`: the observed figure the implied one is set beside
    /// (`docs/SPEC.md` §6.6 counts the settled ones). `None` for an order
    /// the relays no longer have, and for one that never settled — a fee
    /// against a canceled order says nothing about volume.
    pub settled_amount_sats: Option<i64>,
}

/// One completed order, seen from the coverage side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub order_id: String,
    pub instance: String,
    pub success_at: i64,
    /// Whether at least one fee names this order.
    pub has_fee: bool,
    /// Whether the instance charged a fee at the time — `Some(false)` when
    /// its kind 38385 in force said `fee = 0`, `None` when none in force
    /// said. §6.6 counts coverage over instances with `fee > 0` only, so
    /// both leave the denominator: an instance charging nothing owes no dev
    /// fee, and one whose policy is unknown cannot be said to owe one
    /// without inferring it — and every figure here is observed.
    pub charges_fee: Option<bool>,
}

/// Everything the §6.6 figures are computed from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DevFeeData {
    pub fees: Vec<Fee>,
    pub settlements: Vec<Settlement>,
}

/// The ways `stats dev-fees --by` can slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Instance,
    /// Calendar months inside the window, each reported as its own window.
    Month,
}

/// The §6.6 figures for one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DevFees {
    /// Sats sent, duplicates excluded.
    pub total_sats: i64,
    /// Fees that count, i.e. one per order.
    pub paid: u64,
    /// Completed orders with a fee over completed orders known to owe one
    /// — the instance's fee in force was above zero; `None` when none is
    /// known to.
    pub coverage: Option<f64>,
    /// `fee.created_at − success_at`, over fees whose order is known and
    /// completed; `None` when there are none.
    pub latency_p50: Option<i64>,
    pub latency_p90: Option<i64>,
    /// Orders paid for more than once.
    pub duplicates: u64,
    /// Fees naming an order never seen.
    pub orphans: u64,
}

/// The §6.6 figures for `data` over `window`.
pub fn summarise(data: &DevFeeData, window: Window) -> DevFees {
    let doubly_paid: HashSet<&str> = data
        .fees
        .iter()
        .filter(|fee| fee.is_duplicate)
        .map(|fee| fee.order_id.as_str())
        .collect();

    let canonical = canonical(data, window);

    let latencies: Vec<i64> = canonical
        .iter()
        .filter_map(|fee| fee.settled_at.map(|settled| fee.created_at - settled))
        .collect();

    let owed: Vec<&Settlement> = data
        .settlements
        .iter()
        .filter(|settlement| window.contains(settlement.success_at))
        .filter(|settlement| settlement.charges_fee == Some(true))
        .collect();
    let covered = owed.iter().filter(|settlement| settlement.has_fee).count();

    DevFees {
        total_sats: canonical.iter().map(|fee| fee.amount_sats).sum(),
        paid: canonical.len() as u64,
        coverage: (!owed.is_empty()).then(|| covered as f64 / owed.len() as f64),
        latency_p50: percentile(&latencies, 0.5),
        latency_p90: percentile(&latencies, 0.9),
        duplicates: canonical
            .iter()
            .filter(|fee| doubly_paid.contains(fee.order_id.as_str()))
            .count() as u64,
        orphans: canonical.iter().filter(|fee| !fee.order_known).count() as u64,
    }
}

/// The fees that count in `window`: one per order — duplicates out — and
/// dated by the fee event.
pub(crate) fn canonical(data: &DevFeeData, window: Window) -> Vec<&Fee> {
    data.fees
        .iter()
        .filter(|fee| !fee.is_duplicate && window.contains(fee.created_at))
        .collect()
}

/// `data` split by instance label, keys in sorted order.
pub fn by_instance(data: &DevFeeData) -> BTreeMap<String, DevFeeData> {
    let mut groups: BTreeMap<String, DevFeeData> = BTreeMap::new();

    for fee in &data.fees {
        groups
            .entry(fee.instance.clone())
            .or_default()
            .fees
            .push(fee.clone());
    }
    for settlement in &data.settlements {
        groups
            .entry(settlement.instance.clone())
            .or_default()
            .settlements
            .push(settlement.clone());
    }

    groups
}

/// The report for `stats dev-fees --by <dimension>`, or the global one when
/// `dimension` is `None`. Names follow [`crate::activity::report`]:
/// `dev_fees.total_sats`, `dev_fees.<instance>.total_sats`,
/// `dev_fees.2026-08.total_sats`. Every block is the seven observed
/// figures and then the three of [`implied`], which need `dev_fee_pct`:
/// the share to assume for an instance's pubkey.
pub fn report(
    data: &DevFeeData,
    window: Window,
    dimension: Option<Dimension>,
    dev_fee_pct: &dyn Fn(&str) -> f64,
) -> Vec<Metric> {
    let block = |prefix: &str, data: &DevFeeData, window: Window| {
        let mut block = metrics(prefix, &summarise(data, window));
        block.extend(implied::metrics(
            prefix,
            &implied::summarise(data, window, dev_fee_pct),
        ));
        block
    };

    match dimension {
        None => block("dev_fees", data, window),
        Some(Dimension::Month) => window
            .months()
            .into_iter()
            .flat_map(|(key, month)| block(&format!("dev_fees.{key}"), data, month))
            .collect(),
        Some(Dimension::Instance) => by_instance(data)
            .into_iter()
            .flat_map(|(key, group)| block(&format!("dev_fees.{key}"), &group, window))
            .collect(),
    }
}

/// One [`DevFees`] as the seven metrics of §6.6, all observed.
pub fn metrics(prefix: &str, dev_fees: &DevFees) -> Vec<Metric> {
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);
    let seconds = |value: Option<i64>| value.map_or(Value::Missing, Value::Seconds);

    vec![
        observed("total_sats", Value::Sats(dev_fees.total_sats)),
        observed("paid", Value::Count(dev_fees.paid as i64)),
        observed(
            "coverage",
            dev_fees.coverage.map_or(Value::Missing, Value::ratio),
        ),
        observed("latency_p50", seconds(dev_fees.latency_p50)),
        observed("latency_p90", seconds(dev_fees.latency_p90)),
        observed("duplicates", Value::Count(dev_fees.duplicates as i64)),
        observed("orphans", Value::Count(dev_fees.orphans as i64)),
    ]
}

pub mod implied;

#[cfg(test)]
mod tests;
