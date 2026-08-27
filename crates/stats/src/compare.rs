//! The instance comparison — view 3 of `docs/SPEC.md` §6.10.
//!
//! One row per instance: completed, volume, completion rate, fee, dev fees
//! sent, dispute rate, version. A thin assembly: every figure is one the
//! families already compute for a slice, and the profile supplies the two
//! that are not figures at all.

use crate::activity::{self, Order};
use crate::dev_fees::{self, DevFeeData};
use crate::disputes::{self, DisputeData};
use crate::instances::Profile;
use crate::metric::{Metric, Value};
use crate::volume;
use crate::window::Window;

/// The figures of one row, in the order a table shows them. The metric
/// names are `compare.<instance>.<column>`.
pub const COLUMNS: [&str; 7] = [
    "completed",
    "volume_sats",
    "completion_rate",
    "fee",
    "dev_fees_sats",
    "dispute_rate",
    "version",
];

/// The comparison, one block per profile in the order given.
///
/// `orders`, `fees` and `disputes` are the whole network's; each block
/// takes its own by instance label, which is what the loaders put on every
/// row and what [`Profile::label`] holds. `disputes` is `None` when the
/// scope cannot reach them (a `--network` narrowing: dispute events carry
/// no network tag), and every dispute rate is then missing.
pub fn report(
    profiles: &[Profile],
    orders: &[Order],
    fees: &DevFeeData,
    disputes: Option<&DisputeData>,
    window: Window,
    now: i64,
) -> Vec<Metric> {
    let fees_by_instance = dev_fees::by_instance(fees);
    let disputes_by_instance = disputes.map(disputes::by_instance);

    profiles
        .iter()
        .flat_map(|profile| {
            let prefix = format!("compare.{}", profile.label);
            let own: Vec<Order> = orders
                .iter()
                .filter(|order| order.instance == profile.label)
                .cloned()
                .collect();
            let activity = activity::summarise(&own, window, now);
            let sent = fees_by_instance
                .get(&profile.label)
                .map_or(0, |fees| dev_fees::summarise(fees, window).total_sats);
            let dispute_rate = disputes_by_instance
                .as_ref()
                .and_then(|by_instance| by_instance.get(&profile.label))
                .and_then(|disputes| disputes::summarise(disputes, window, now).rate);

            let observed =
                |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);

            vec![
                observed("completed", Value::Count(activity.completed as i64)),
                observed(
                    "volume_sats",
                    Value::Sats(volume::observed_sats(&own, window)),
                ),
                observed(
                    "completion_rate",
                    activity
                        .completion_rate
                        .map_or(Value::Missing, Value::ratio),
                ),
                observed("fee", profile.fee.map_or(Value::Missing, Value::ratio)),
                observed("dev_fees_sats", Value::Sats(sent)),
                observed(
                    "dispute_rate",
                    dispute_rate.map_or(Value::Missing, Value::ratio),
                ),
                observed(
                    "version",
                    profile
                        .mostro_version
                        .clone()
                        .map_or(Value::Missing, Value::Text),
                ),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests;
