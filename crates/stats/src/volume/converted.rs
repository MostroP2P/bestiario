//! Volume in a reference currency — `docs/SPEC.md` §6.2's inferred row and
//! roadmap PR 35.
//!
//! `∑ amount_sats × rate(code, ≤ success_at)` over the orders that reached
//! `success` in the window, each at the rate the instance that settled it
//! had published by then ([`RateBook::rate_at`]). The figure is inferred:
//! the trade was in some fiat at some price, and this values its sats at a
//! *different* currency's price as one instance saw it, minutes before.
//! What qualifies it travels with it, as §5 requires — the age of the
//! oldest quote used, how many orders were priced on another instance's
//! snapshot because their own had none, and how many could not be priced
//! at all and are left out of the sum.

use crate::activity::Order;
use crate::metric::{Metric, Value};
use crate::rates::{RateBook, RateSource};
use crate::window::Window;

/// The conversion of one window into one currency.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The reference currency.
    pub code: String,
    /// `∑ amount_sats × rate` over the priced orders.
    pub total: f64,
    /// Completed orders that had a rate at or before `success_at`.
    pub priced: u64,
    /// Completed orders that had none; not in `total`.
    pub unpriced: u64,
    /// Their sats, so the reader can see the size of the hole.
    pub unpriced_sats: i64,
    /// Of the priced, how many used another instance's snapshot.
    pub fallbacks: u64,
    /// The oldest quote used; `None` when nothing was priced.
    pub rate_age_max_secs: Option<i64>,
}

/// Values the orders completed in `window` in `code`, order by order.
pub fn convert(orders: &[Order], window: Window, book: &RateBook, code: &str) -> Converted {
    let mut converted = Converted {
        code: code.to_string(),
        total: 0.0,
        priced: 0,
        unpriced: 0,
        unpriced_sats: 0,
        fallbacks: 0,
        rate_age_max_secs: None,
    };

    for order in super::completed(orders, window) {
        let settled_at = order
            .success_at
            .expect("completed orders have a success_at");
        match book.rate_at(&order.pubkey, code, settled_at) {
            Some(quote) => {
                converted.total += quote.convert_sats(order.amount_sats);
                converted.priced += 1;
                if matches!(quote.source, RateSource::Fallback { .. }) {
                    converted.fallbacks += 1;
                }
                converted.rate_age_max_secs = Some(
                    converted
                        .rate_age_max_secs
                        .map_or(quote.age_secs, |age| age.max(quote.age_secs)),
                );
            }
            None => {
                converted.unpriced += 1;
                converted.unpriced_sats += order.amount_sats;
            }
        }
    }

    converted
}

/// One [`Converted`] as metrics under `{prefix}.in.{code}`, every one of
/// them inferred and carrying what qualifies it.
pub fn metrics(prefix: &str, converted: &Converted) -> Vec<Metric> {
    let code = &converted.code;
    let inferred = |name: &str, value: Value, error: String| {
        Metric::inferred(format!("{prefix}.in.{code}.{name}"), value, error)
    };

    // Orders to price but none priced: the absence of an answer, not zero.
    let total = if converted.priced == 0 && converted.unpriced > 0 {
        Value::Missing
    } else {
        Value::fiat(converted.total, code.clone())
    };

    vec![
        inferred("total", total, qualification(converted)),
        inferred(
            "orders",
            Value::Count(converted.priced as i64),
            "orders with a rate published at or before success_at".to_string(),
        ),
        inferred(
            "unpriced_sats",
            Value::Sats(converted.unpriced_sats),
            "sats of the orders no instance had a rate for by success_at; not in the total"
                .to_string(),
        ),
        inferred(
            "rate_age_max",
            converted
                .rate_age_max_secs
                .map_or(Value::Missing, Value::Seconds),
            "age of the oldest snapshot used".to_string(),
        ),
    ]
}

/// The error column of the total: the rate age first, since every
/// conversion has one, then whatever else weakens the figure.
fn qualification(converted: &Converted) -> String {
    let mut parts = vec![match converted.rate_age_max_secs {
        Some(age) => format!("rate_age_secs ≤ {age}"),
        None => "no rate used".to_string(),
    }];
    if converted.fallbacks > 0 {
        parts.push(format!(
            "{} at another instance's rate",
            converted.fallbacks
        ));
    }
    if converted.unpriced > 0 {
        parts.push(format!(
            "{} unpriced ({} sats excluded)",
            converted.unpriced, converted.unpriced_sats
        ));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests;
