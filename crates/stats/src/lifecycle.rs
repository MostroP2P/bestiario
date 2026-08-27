//! The lifecycle of one order — `bestiario orders <ORDER_ID>`.
//!
//! Every version in chronological order, then every dev fee that names the
//! order. Not an aggregation: this is the one view that shows the events
//! themselves, which is the payoff for persisting every version (SPEC §4).
//! The structs are plain so that the view can be served without this crate
//! seeing the parser or the database.

use chrono::{DateTime, Utc};

use crate::activity::{Direction, Status};
use crate::metric::{Metric, Value};

/// The fiat side of one version.
#[derive(Debug, Clone, PartialEq)]
pub enum Fiat {
    Fixed(f64),
    Range { min: f64, max: f64 },
}

/// One published version of the order.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub at: i64,
    pub status: Status,
    pub direction: Direction,
    pub fiat_code: String,
    pub amount_sats: i64,
    pub fiat: Fiat,
    pub premium: f64,
    pub expires_at: i64,
}

/// One dev fee naming the order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeSeen {
    pub at: i64,
    pub amount_sats: i64,
    pub is_duplicate: bool,
}

/// The view: `order.<n>.*` per version, oldest first, then
/// `dev_fee.<n>.*` per fee.
pub fn report(order_id: &str, versions: &[Version], fees: &[FeeSeen]) -> Vec<Metric> {
    let mut metrics = vec![
        Metric::observed("order.id", Value::Text(order_id.to_string())),
        Metric::observed("order.versions", Value::Count(versions.len() as i64)),
    ];

    for (index, version) in versions.iter().enumerate() {
        let prefix = format!("order.{}", index + 1);
        let observed =
            |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);

        metrics.extend([
            observed("at", Value::Text(rfc3339(version.at))),
            observed("status", Value::Text(version.status.as_str().to_string())),
            observed("kind", Value::Text(version.direction.as_str().to_string())),
            observed("amount", Value::Sats(version.amount_sats)),
            observed(
                "fiat",
                match &version.fiat {
                    Fiat::Fixed(amount) => Value::fiat(*amount, version.fiat_code.clone()),
                    Fiat::Range { min, max } => {
                        Value::Text(format!("{min:.2}–{max:.2} {}", version.fiat_code))
                    }
                },
            ),
            observed("premium", Value::ratio(version.premium / 100.0)),
            observed("expires_at", Value::Text(rfc3339(version.expires_at))),
        ]);
    }

    for (index, fee) in fees.iter().enumerate() {
        let prefix = format!("dev_fee.{}", index + 1);
        metrics.extend([
            Metric::observed(format!("{prefix}.at"), Value::Text(rfc3339(fee.at))),
            Metric::observed(format!("{prefix}.amount"), Value::Sats(fee.amount_sats)),
            Metric::observed(
                format!("{prefix}.duplicate"),
                Value::Text(if fee.is_duplicate { "yes" } else { "no" }.to_string()),
            ),
        ]);
    }

    metrics
}

fn rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|at| at.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

#[cfg(test)]
mod tests;
