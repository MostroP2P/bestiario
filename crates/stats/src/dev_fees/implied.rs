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
//! The comparison of §6.6 sets the implied volume beside the observed one
//! over the fees whose order is known — the only ones with an observed
//! side. Fees for unseen orders (the usual case in a backfill, since fees
//! outlive orders on the relays) are in the implied total and not in the
//! comparison, which is the point: they are the volume the orders no longer
//! show.

use super::{DevFeeData, Fee};
use crate::metric::{Metric, Value};
use crate::window::Window;

/// The inverse, and what qualifies it, for one window.
#[derive(Debug, Clone, PartialEq)]
pub struct Implied {
    /// `∑ dev_fee / (fee × pct)` over the invertible fees, rounded; `None`
    /// when there were fees and none could be inverted.
    pub volume_sats: Option<i64>,
    /// `∑ 1 / (fee × pct)`, rounded up: the volume one sat of rounding per
    /// fee amounts to.
    pub error_sats: i64,
    /// Fees inverted.
    pub inverted: u64,
    /// Fees with no fee in force, or a zero one, which no inverse fits.
    pub not_invertible: u64,
    /// The distinct `pct` values assumed, ascending.
    pub assumed_pcts: Vec<f64>,
    /// Of the inverted, those whose order is known.
    pub matched: u64,
    /// Their implied volume, rounded.
    pub matched_implied_sats: i64,
    /// `∑ amount_sats` of their orders — the observed side.
    pub matched_observed_sats: i64,
}

impl Implied {
    /// `implied / observed − 1` over the matched fees: how far the assumed
    /// share is from the real one. Positive means the real share is above
    /// the assumed. `None` with nothing matched.
    pub fn implied_vs_observed(&self) -> Option<f64> {
        (self.matched_observed_sats > 0)
            .then(|| self.matched_implied_sats as f64 / self.matched_observed_sats as f64 - 1.0)
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
        not_invertible: 0,
        assumed_pcts: vec![],
        matched: 0,
        matched_implied_sats: 0,
        matched_observed_sats: 0,
    };
    let mut volume = 0.0;
    let mut error = 0.0;
    let mut matched_volume = 0.0;
    let mut pcts: Vec<f64> = vec![];

    for fee in super::canonical(data, window) {
        let pct = dev_fee_pct(&fee.pubkey);
        let Some(divisor) = divisor(fee, pct) else {
            implied.not_invertible += 1;
            continue;
        };
        if !pcts.contains(&pct) {
            pcts.push(pct);
        }

        let inverse = fee.amount_sats as f64 / divisor;
        volume += inverse;
        error += 1.0 / divisor;
        implied.inverted += 1;
        if let Some(observed) = fee.order_amount_sats {
            implied.matched += 1;
            matched_volume += inverse;
            implied.matched_observed_sats += observed;
        }
    }

    pcts.sort_by(|a, b| a.partial_cmp(b).expect("a share is finite"));
    implied.assumed_pcts = pcts;
    implied.error_sats = error.ceil() as i64;
    implied.matched_implied_sats = matched_volume.round() as i64;
    implied.volume_sats = match (implied.inverted, implied.not_invertible) {
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
            Value::Sats(implied.matched_observed_sats),
        ),
        Metric::inferred(
            name("implied_vs_observed"),
            implied
                .implied_vs_observed()
                .map_or(Value::Missing, Value::ratio),
            if implied.matched == 0 {
                "no inverted fee names a known order".to_string()
            } else {
                format!(
                    "implied / observed − 1 over the {} fees whose order is known; \
                     positive means the real dev fee share is above the assumed",
                    implied.matched
                )
            },
        ),
    ]
}

/// The error column of the implied volume: the bound, the assumption,
/// and what was left out.
fn qualification(implied: &Implied) -> String {
    let mut parts = vec![if implied.inverted == 0 {
        "no fee inverted".to_string()
    } else {
        format!(
            "±{} sats (±1 sat per fee × 1/(fee×pct))",
            implied.error_sats
        )
    }];
    if !implied.assumed_pcts.is_empty() {
        let pcts: Vec<String> = implied
            .assumed_pcts
            .iter()
            .map(|pct| format!("{pct:.2}"))
            .collect();
        parts.push(format!("pct assumed {}", pcts.join("/")));
    }
    if implied.not_invertible > 0 {
        parts.push(format!(
            "{} fees not invertible (no fee in force)",
            implied.not_invertible
        ));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests;
