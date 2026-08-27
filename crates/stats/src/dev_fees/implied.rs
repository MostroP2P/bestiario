//! Volume implied by dev fees — `docs/SPEC.md` §5 and the implied-vs-
//! observed row of §6.6; roadmap PR 36.
//!
//! mostrod computes the dev fee as `round(fee × amount × pct)`, so a fee
//! of `d` sats was produced by an order of about `d / (fee × pct)` sats.
//! Two things qualify the inverse and travel with it. The rounding: `d` is
//! within a sat of the true product, and one sat of `d` is `1 / (fee × pct)`
//! sats of volume — with `fee = 0.006` and `pct = 0.30`, some 550 sats per
//! fee. And the share itself: `pct` is not published by any instance, so
//! it is an assumption from `settings.toml`, and every figure here says
//! which one it rested on.
//!
//! A third thing qualifies the total and is the largest of the three in a
//! backfill: the fees whose instance never published a `fee` the projection
//! still has. They cannot be inverted, they are not in the sum, and no
//! rounding bound covers them — so whenever there is one the figure is a
//! **lower bound** and says so.
//!
//! The comparison of §6.6 sets the implied volume beside the observed one.
//! The observed side is `∑ amount_sats` of the *settled* orders a fee names
//! (§6.6: "∑ `amount_sats` of `success` with dev fee") and is what it says —
//! it does not move with what can be inverted. The ratio is over the
//! intersection: the fees that could be inverted *and* whose settled order
//! carries an amount to compare against. Fees for unseen orders (the usual
//! case in a backfill, since fees outlive orders on the relays) are in the
//! implied total and in neither of the other two, which is the point: they
//! are the volume the orders no longer show.

use super::{DevFeeData, Fee};
use crate::metric::{Metric, Value};
use crate::window::Window;

/// Distinct assumed shares listed one by one before the error column
/// summarises them as a range instead.
const SHARES_LISTED: usize = 3;

/// The inverse, and what qualifies it, for one window.
#[derive(Debug, Clone, PartialEq)]
pub struct Implied {
    /// `∑ dev_fee / (fee × pct)` over the invertible fees, rounded; `None`
    /// when there were fees and none could be inverted. A lower bound
    /// whenever [`not_invertible`](Self::not_invertible) is not zero.
    pub volume_sats: Option<i64>,
    /// `∑ 1 / (fee × pct)`, rounded up: the volume one sat of rounding per
    /// fee amounts to, every fee wrong in the same direction — the worst
    /// case, not the likely one.
    pub error_sats: i64,
    /// Fees inverted.
    pub inverted: u64,
    /// Fees whose instance has no `fee` in force to divide by.
    pub no_fee_in_force: u64,
    /// Fees whose instance had a `fee` of zero in force, which no inverse
    /// fits — an instance that charges nothing, not a gap in the data.
    pub zero_fee: u64,
    /// The distinct `pct` values assumed, ascending.
    pub assumed_pcts: Vec<f64>,
    /// `∑ amount_sats` of the settled orders the fees name — the observed
    /// side, independent of what could be inverted. `None` when no fee
    /// names a settled order, which is not the same as zero volume.
    pub with_fee_volume_sats: Option<i64>,
    /// The settled orders that sum to it.
    pub with_fee_orders: u64,
    /// Fees that are in both sides of the ratio: inverted, and naming a
    /// settled order of a positive amount.
    pub matched: u64,
    /// Their implied volume, rounded.
    pub matched_implied_sats: i64,
    /// `∑ amount_sats` of their orders.
    pub matched_observed_sats: i64,
    /// The rounding bound over those same fees.
    pub matched_error_sats: i64,
    /// Fees whose settled order is known to the projection at `amt = 0` —
    /// a market-price order no later version amended (`docs/SPEC.md` §3).
    /// Left out of the ratio, where a zero denominator is not a datum.
    pub zero_amount_orders: u64,
}

impl Implied {
    /// Fees no inverse fits, for either reason.
    pub fn not_invertible(&self) -> u64 {
        self.no_fee_in_force + self.zero_fee
    }

    /// `implied / observed − 1` over the matched fees: how far the assumed
    /// share is from the real one. Positive means the real share is above
    /// the assumed. `None` with nothing matched.
    pub fn implied_vs_observed(&self) -> Option<f64> {
        (self.matched_observed_sats > 0)
            .then(|| self.matched_implied_sats as f64 / self.matched_observed_sats as f64 - 1.0)
    }

    /// The worst-case rounding on that ratio: the bound on the numerator
    /// over the observed denominator.
    pub fn implied_vs_observed_error(&self) -> Option<f64> {
        (self.matched_observed_sats > 0)
            .then(|| self.matched_error_sats as f64 / self.matched_observed_sats as f64)
    }
}

/// The inverse over the fees of `data` counted in `window` — the same
/// fees `total_sats` sums, one per order — with `dev_fee_pct` giving the
/// share to assume for an instance's pubkey.
pub fn summarise(data: &DevFeeData, window: Window, dev_fee_pct: &dyn Fn(&str) -> f64) -> Implied {
    let mut implied = Implied {
        volume_sats: Some(0),
        error_sats: 0,
        inverted: 0,
        no_fee_in_force: 0,
        zero_fee: 0,
        assumed_pcts: vec![],
        with_fee_volume_sats: None,
        with_fee_orders: 0,
        matched: 0,
        matched_implied_sats: 0,
        matched_observed_sats: 0,
        matched_error_sats: 0,
        zero_amount_orders: 0,
    };
    let mut volume = 0.0;
    let mut error = 0.0;
    let mut matched_volume = 0.0;
    let mut matched_error = 0.0;
    let mut observed = 0i64;
    let mut pcts: Vec<f64> = vec![];

    for fee in super::canonical(data, window) {
        // The observed side first: it is the settled orders themselves, and
        // whether the fee can be inverted has no say in it.
        if let Some(amount) = fee.settled_amount_sats {
            implied.with_fee_orders += 1;
            observed += amount;
        }

        let pct = dev_fee_pct(&fee.pubkey);
        let Some(divisor) = divisor(fee, pct) else {
            match fee.fee_in_force {
                None => implied.no_fee_in_force += 1,
                Some(_) => implied.zero_fee += 1,
            }
            continue;
        };
        if !pcts.contains(&pct) {
            pcts.push(pct);
        }

        let inverse = fee.amount_sats as f64 / divisor;
        volume += inverse;
        error += 1.0 / divisor;
        implied.inverted += 1;

        match fee.settled_amount_sats {
            Some(amount) if amount > 0 => {
                implied.matched += 1;
                matched_volume += inverse;
                matched_error += 1.0 / divisor;
                implied.matched_observed_sats += amount;
            }
            Some(_) => implied.zero_amount_orders += 1,
            None => {}
        }
    }

    pcts.sort_by(f64::total_cmp);
    implied.assumed_pcts = pcts;
    implied.error_sats = error.ceil() as i64;
    implied.matched_implied_sats = matched_volume.round() as i64;
    implied.matched_error_sats = matched_error.ceil() as i64;
    implied.with_fee_volume_sats = (implied.with_fee_orders > 0).then_some(observed);
    implied.volume_sats = match (implied.inverted, implied.not_invertible()) {
        (0, stuck) if stuck > 0 => None,
        _ => Some(volume.round() as i64),
    };

    implied
}

/// `fee × pct` when it can be divided by.
fn divisor(fee: &Fee, pct: f64) -> Option<f64> {
    let divisor = fee.fee_in_force? * pct;
    (divisor.is_finite() && divisor > 0.0).then_some(divisor)
}

/// One [`Implied`] as the three rows of the comparison: the inferred
/// volume, the observed one it is set beside, and their ratio.
pub fn metrics(prefix: &str, implied: &Implied) -> Vec<Metric> {
    let name = |name: &str| format!("{prefix}.{name}");

    vec![
        Metric::inferred(
            name("implied_volume"),
            implied.volume_sats.map_or(Value::Missing, Value::Sats),
            qualification(implied),
        ),
        Metric::observed(
            name("with_fee_volume"),
            implied
                .with_fee_volume_sats
                .map_or(Value::Missing, Value::Sats),
        ),
        Metric::inferred(
            name("implied_vs_observed"),
            implied
                .implied_vs_observed()
                .map_or(Value::Missing, Value::ratio),
            comparison(implied),
        ),
    ]
}

/// The error column of the implied volume: the rounding bound, the
/// assumption, and — the part that dwarfs both in a backfill — the fees
/// left out, which make the figure a lower bound.
fn qualification(implied: &Implied) -> String {
    let mut parts = vec![if implied.inverted == 0 {
        "no fee inverted".to_string()
    } else {
        format!(
            "±{} sats (worst case: ±1 sat per fee × 1/(fee×pct))",
            implied.error_sats
        )
    }];
    if !implied.assumed_pcts.is_empty() {
        parts.push(format!("pct assumed {}", shares(&implied.assumed_pcts)));
    }
    if implied.not_invertible() > 0 {
        let total = implied.inverted + implied.not_invertible();
        let lower_bound = if implied.inverted == 0 {
            String::new()
        } else {
            format!(
                "lower bound: {} of {total} fees inverted; ",
                implied.inverted
            )
        };
        parts.push(format!("{lower_bound}{}", uninvertible(implied)));
    }
    parts.join("; ")
}

/// Why the fees that were left out could not be inverted — a missing rate
/// and an instance that charges nothing are not the same gap.
fn uninvertible(implied: &Implied) -> String {
    let mut reasons = vec![];
    if implied.no_fee_in_force > 0 {
        reasons.push(format!(
            "{} with no fee in force",
            fees(implied.no_fee_in_force)
        ));
    }
    if implied.zero_fee > 0 {
        reasons.push(format!("{} charging a zero fee", fees(implied.zero_fee)));
    }
    reasons.join(", ")
}

/// The error column of the ratio: what it is over, its own rounding bound,
/// and how to read it.
fn comparison(implied: &Implied) -> String {
    let Some(error) = implied.implied_vs_observed_error() else {
        return match (implied.zero_amount_orders, implied.with_fee_orders) {
            (0, 0) => "no fee names a settled order".to_string(),
            (0, _) => "no fee both inverted and naming a settled order".to_string(),
            (zero, _) => format!(
                "nothing to compare against: {} name a settled order known at \
                 amt = 0 (market price, never amended)",
                fees(zero)
            ),
        };
    };

    let mut parts = vec![
        format!(
            "implied / observed − 1 over the {} whose order is known and settled; \
             positive means the real dev fee share is above the assumed",
            fees(implied.matched)
        ),
        format!("±{error:.4} from the rounding of those fees"),
    ];
    if implied.zero_amount_orders > 0 {
        parts.push(format!(
            "{} left out at amt = 0 (market price, never amended)",
            fees(implied.zero_amount_orders)
        ));
    }
    parts.join("; ")
}

/// `n fees`, or `1 fee`.
fn fees(count: u64) -> String {
    match count {
        1 => "1 fee".to_string(),
        n => format!("{n} fees"),
    }
}

/// The assumed shares, listed while they are few and given as a range once
/// a network's worth of overrides would make a list of them unreadable.
fn shares(pcts: &[f64]) -> String {
    match (pcts.len(), pcts.first(), pcts.last()) {
        (len, Some(low), Some(high)) if len > SHARES_LISTED => {
            format!("{low:.2}–{high:.2} across {len} instances")
        }
        _ => pcts
            .iter()
            .map(|pct| format!("{pct:.2}"))
            .collect::<Vec<String>>()
            .join("/"),
    }
}

#[cfg(test)]
mod tests;
