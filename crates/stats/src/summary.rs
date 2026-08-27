//! The network summary — view 1 of `docs/SPEC.md` §6.10.
//!
//! One screen for the period: created, completed, completion rate, sats
//! volume, active instances, top fiat currencies, top payment methods, open
//! disputes. Nothing here is computed for the first time; the view picks
//! from the families and adds the two rankings and the instance count.

use std::collections::BTreeMap;

use crate::activity::{self, Order};
use crate::disputes::{self, DisputeData};
use crate::metric::{Metric, Value};
use crate::volume;
use crate::window::Window;

/// How many currencies and payment methods the summary names.
pub const TOP_N: usize = 3;

/// The summary for `orders` and `disputes` over `window`.
///
/// `disputes` is `None` when the scope cannot reach them — a `--network`
/// narrowing, which dispute events carry no tag for — and the open-dispute
/// count is then missing rather than network-wide under a scoped heading.
pub fn report(
    orders: &[Order],
    disputes: Option<&DisputeData>,
    window: Window,
    now: i64,
) -> Vec<Metric> {
    let activity = activity::summarise(orders, window, now);
    let open_disputes =
        disputes.map(|disputes| disputes::summarise(disputes, window, now).open.len());

    let created: Vec<&Order> = orders
        .iter()
        .filter(|order| window.contains(order.created_at))
        .collect();
    let active_instances = created
        .iter()
        .map(|order| order.pubkey.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let observed = |name: &str, value: Value| Metric::observed(format!("summary.{name}"), value);

    vec![
        observed("created", Value::Count(activity.created as i64)),
        observed("completed", Value::Count(activity.completed as i64)),
        observed(
            "completion_rate",
            activity
                .completion_rate
                .map_or(Value::Missing, Value::ratio),
        ),
        observed(
            "volume_sats",
            Value::Sats(volume::observed_sats(orders, window)),
        ),
        observed("active_instances", Value::Count(active_instances as i64)),
        observed(
            "top_fiat",
            ranking(created.iter().map(|order| order.fiat_code.as_str())),
        ),
        observed(
            "top_methods",
            ranking(
                created
                    .iter()
                    .flat_map(|order| order.payment_methods.iter().map(String::as_str)),
            ),
        ),
        observed(
            "open_disputes",
            open_disputes.map_or(Value::Missing, |open| Value::Count(open as i64)),
        ),
    ]
}

/// The [`TOP_N`] most frequent keys with their counts, most frequent first,
/// as `ARS (12), VES (7), USD (3)`; missing when there is nothing to rank.
///
/// Ties break alphabetically, so the same data always ranks the same way.
fn ranking<'a>(keys: impl Iterator<Item = &'a str>) -> Value {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for key in keys {
        *counts.entry(key).or_default() += 1;
    }
    if counts.is_empty() {
        return Value::Missing;
    }

    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    Value::Text(
        ranked
            .iter()
            .take(TOP_N)
            .map(|(key, count)| format!("{key} ({count})"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

#[cfg(test)]
mod tests;
