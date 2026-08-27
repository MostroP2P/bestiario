//! The bestiary — `docs/SPEC.md` §6.5 — and the instance profile view
//! (§6.10, view 2).
//!
//! An instance's profile is what it published about itself in its latest
//! kind 38385, plus when it was first and last heard from. The list report
//! is one block of those per instance, with whether it created anything in
//! the window and how long it has been silent. The profile view adds the
//! instance's own §6.1, §6.6 and §6.7 figures and its share of the network.
//!
//! # Silence
//!
//! "Silent" is a claim about the clock — no event of any kind for longer
//! than [`SILENT_AFTER_SECS`] — so it is reported as a duration, and the
//! threshold is a constant here rather than an option: it is a definition
//! of the word, and a report where each reader picks their own would not be
//! comparable across runs.

use chrono::{DateTime, Utc};

use crate::activity::{self, Order};
use crate::dev_fees::{self, DevFeeData};
use crate::disputes::{self, DisputeData};
use crate::metric::{Metric, Value};
use crate::volume;
use crate::window::Window;

/// How long without any event before an instance is called silent: a week.
///
/// Orders expire in a day and a live instance republishes its info; a week
/// of nothing is not a quiet day.
pub const SILENT_AFTER_SECS: i64 = 7 * 86_400;

/// What is known of one instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub pubkey: String,
    pub name: Option<String>,
    /// `name (short pubkey)`, or the pubkey — the same label the slices use.
    pub label: String,
    pub mostro_version: Option<String>,
    pub protocol_version: Option<String>,
    /// Per-side fee as a fraction, from the latest 38385 that said.
    pub fee: Option<f64>,
    pub min_order_sats: Option<i64>,
    pub max_order_sats: Option<i64>,
    pub fiat_currencies: Vec<String>,
    pub ln_networks: Vec<String>,
    pub bond_enabled: Option<bool>,
    /// `created_at` of the first and last event of any kind.
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

impl Profile {
    /// Seconds since the last event of any kind.
    pub fn silent_for(&self, now: i64) -> i64 {
        now - self.last_seen_at
    }

    pub fn is_silent(&self, now: i64) -> bool {
        self.silent_for(now) > SILENT_AFTER_SECS
    }
}

/// The report for `bestiario instances`: one block per profile, in the
/// order given, each with the orders it created in the window.
///
/// `orders` is the whole network's; each block counts its own by pubkey.
pub fn list(profiles: &[Profile], orders: &[Order], window: Window, now: i64) -> Vec<Metric> {
    profiles
        .iter()
        .flat_map(|profile| {
            let prefix = format!("instances.{}", profile.label);
            let created = orders
                .iter()
                .filter(|order| order.pubkey == profile.pubkey)
                .filter(|order| window.contains(order.created_at))
                .count();

            let mut metrics = profile_metrics(&prefix, profile, now);
            metrics.push(Metric::observed(
                format!("{prefix}.created"),
                Value::Count(created as i64),
            ));
            metrics
        })
        .collect()
}

/// The report for `bestiario instance <PUBKEY|NAME>`: the profile, the
/// instance's own §6.1, §6.6 and §6.7 figures, and its share of the
/// network's orders and volume in the window.
///
/// `own` are the instance's orders and `network` everyone's, the instance
/// included; the shares are `own / network`, or missing when the network
/// did nothing.
pub fn profile(
    profile: &Profile,
    own: &[Order],
    network: &[Order],
    fees: &DevFeeData,
    disputes: &DisputeData,
    window: Window,
    now: i64,
) -> Vec<Metric> {
    let mut metrics = profile_metrics("instance", profile, now);

    let activity = activity::summarise(own, window, now);
    metrics.extend(activity::metrics("orders", &activity));
    metrics.push(Metric::observed(
        "volume.sats",
        Value::Sats(volume::observed_sats(own, window)),
    ));
    metrics.extend(dev_fees::metrics(
        "dev_fees",
        &dev_fees::summarise(fees, window),
    ));
    metrics.extend(disputes::metrics(
        "disputes",
        &disputes::summarise(disputes, window, now),
    ));

    let network_activity = activity::summarise(network, window, now);
    metrics.push(Metric::observed(
        "share.orders",
        share(activity.created as f64, network_activity.created as f64),
    ));
    metrics.push(Metric::observed(
        "share.volume",
        share(
            volume::observed_sats(own, window) as f64,
            volume::observed_sats(network, window) as f64,
        ),
    ));

    metrics
}

/// `part / whole` as a ratio, or missing when there is no whole.
fn share(part: f64, whole: f64) -> Value {
    if whole > 0.0 {
        Value::ratio(part / whole)
    } else {
        Value::Missing
    }
}

/// The profile fields of §6.5 as metrics under `prefix`.
pub fn profile_metrics(prefix: &str, profile: &Profile, now: i64) -> Vec<Metric> {
    let text = |name: &str, value: Option<&str>| {
        Metric::observed(
            format!("{prefix}.{name}"),
            value.map_or(Value::Missing, |text| Value::Text(text.to_string())),
        )
    };
    let list = |name: &str, values: &[String]| {
        text(
            name,
            (!values.is_empty()).then(|| values.join(",")).as_deref(),
        )
    };
    let sats = |name: &str, value: Option<i64>| {
        Metric::observed(
            format!("{prefix}.{name}"),
            value.map_or(Value::Missing, Value::Sats),
        )
    };

    vec![
        Metric::observed(
            format!("{prefix}.pubkey"),
            Value::Text(profile.pubkey.clone()),
        ),
        text("name", profile.name.as_deref()),
        text("mostro_version", profile.mostro_version.as_deref()),
        text("protocol_version", profile.protocol_version.as_deref()),
        Metric::observed(
            format!("{prefix}.fee"),
            profile.fee.map_or(Value::Missing, Value::ratio),
        ),
        sats("min_order", profile.min_order_sats),
        sats("max_order", profile.max_order_sats),
        list("fiat", &profile.fiat_currencies),
        list("ln_networks", &profile.ln_networks),
        text(
            "bond",
            profile
                .bond_enabled
                .map(|enabled| if enabled { "enabled" } else { "disabled" }),
        ),
        Metric::observed(
            format!("{prefix}.first_seen"),
            Value::Text(rfc3339(profile.first_seen_at)),
        ),
        Metric::observed(
            format!("{prefix}.last_seen"),
            Value::Text(rfc3339(profile.last_seen_at)),
        ),
        Metric::observed(
            format!("{prefix}.silent_for"),
            Value::Seconds(profile.silent_for(now)),
        ),
        text(
            "silent",
            Some(if profile.is_silent(now) { "yes" } else { "no" }),
        ),
    ]
}

/// A unix timestamp as RFC 3339, or the bare number when out of range.
fn rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|at| at.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

#[cfg(test)]
mod tests;
