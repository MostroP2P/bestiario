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

use std::collections::BTreeMap;

use crate::activity::Order;
use crate::metric::{Metric, Value};
use crate::rates::{RateBook, RateSource};
use crate::window::Window;

/// What one instance's borrowed snapshots priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Borrowed {
    pub orders: u64,
    /// `None` once the sats stop fitting in an `i64`.
    pub sats: Option<i64>,
}

impl Default for Borrowed {
    /// Nothing borrowed yet is zero sats, not an overflowed sum.
    fn default() -> Self {
        Self {
            orders: 0,
            sats: Some(0),
        }
    }
}

/// The conversion of one window into one currency.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The reference currency.
    pub code: String,
    /// `∑ amount_sats × rate` over the priced orders, while that sum stays
    /// finite; `None` once it does not. A figure that left `f64` is not a
    /// smaller or larger figure, it is no figure at all.
    pub total: Option<f64>,
    /// Completed orders that had a usable rate at or before `success_at`
    /// and converted to a finite figure.
    pub priced: u64,
    /// Completed orders with no usable rate; not in `total`.
    pub unpriced: u64,
    /// Their sats, so the reader can see the size of the hole; `None` when
    /// they no longer fit in an `i64`.
    pub unpriced_sats: Option<i64>,
    /// Orders that had a rate but whose conversion is not a finite number.
    /// Kept apart from [`Self::unpriced`]: the data was there and the
    /// arithmetic failed, which is a different thing to go and fix.
    pub unusable: u64,
    /// Their sats.
    pub unusable_sats: Option<i64>,
    /// Per instance whose snapshot was borrowed, what it priced. §5 keeps
    /// the publisher's identity because a bare count cannot say whether one
    /// source or five supplied a window's prices.
    pub borrowed: BTreeMap<String, Borrowed>,
    /// The oldest quote used; `None` when nothing was priced.
    pub rate_age_max_secs: Option<i64>,
}

impl Converted {
    /// How many orders were priced on another instance's snapshot.
    pub fn fallbacks(&self) -> u64 {
        self.borrowed.values().map(|borrowed| borrowed.orders).sum()
    }

    /// `(pubkey, orders, sats)` per borrowed source, by pubkey.
    pub fn fallback_sources(&self) -> Vec<(String, u64, i64)> {
        self.borrowed
            .iter()
            .map(|(pubkey, borrowed)| (pubkey.clone(), borrowed.orders, borrowed.sats.unwrap_or(0)))
            .collect()
    }

    /// The sum over the priced orders, `None` when it is not a number.
    pub fn total_of_priced(&self) -> Option<f64> {
        self.total
    }
}

/// A running `i64` sum that becomes `None` rather than wrapping.
fn add_sats(total: Option<i64>, sats: i64) -> Option<i64> {
    total
        .map(i128::from)
        .map(|total| total + i128::from(sats))
        .and_then(|total| i64::try_from(total).ok())
}

/// The first [`SHORT_PUBKEY`] characters, the way every report names an
/// instance it has no name for.
fn short(pubkey: &str) -> String {
    pubkey.chars().take(SHORT_PUBKEY).collect()
}

/// How many characters of a pubkey a report shows (`docs/SPEC.md` §3).
const SHORT_PUBKEY: usize = 8;

/// Values the orders completed in `window` in `code`, order by order.
pub fn convert(orders: &[Order], window: Window, book: &RateBook, code: &str) -> Converted {
    let mut converted = Converted {
        code: code.to_string(),
        total: Some(0.0),
        priced: 0,
        unpriced: 0,
        unpriced_sats: Some(0),
        unusable: 0,
        unusable_sats: Some(0),
        borrowed: BTreeMap::new(),
        rate_age_max_secs: None,
    };

    for order in super::completed(orders, window) {
        let settled_at = order
            .success_at
            .expect("completed orders have a success_at");
        let Some(quote) = book.rate_at(&order.pubkey, code, settled_at) else {
            converted.unpriced += 1;
            converted.unpriced_sats = add_sats(converted.unpriced_sats, order.amount_sats);
            continue;
        };

        // A rate is checked to be finite and positive when it is parsed,
        // and an amount is a number of satoshis — and their product can
        // still leave `f64`, as can the sum of two that did not. Whichever
        // of the two fails, the figure asked for does not exist, and
        // saying so is not the same as saying the order had no price.
        let value = quote.convert_sats(order.amount_sats);
        if !value.is_finite() {
            converted.unusable += 1;
            converted.unusable_sats = add_sats(converted.unusable_sats, order.amount_sats);
            continue;
        }

        converted.priced += 1;
        converted.total = converted
            .total
            .map(|total| total + value)
            .filter(|total| total.is_finite());
        if let RateSource::Fallback { pubkey } = &quote.source {
            let borrowed = converted.borrowed.entry(pubkey.clone()).or_default();
            borrowed.orders += 1;
            borrowed.sats = add_sats(borrowed.sats, order.amount_sats);
        }
        converted.rate_age_max_secs = Some(
            converted
                .rate_age_max_secs
                .map_or(quote.age_secs, |age| age.max(quote.age_secs)),
        );
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

    // Orders to price but none priced, or a sum that left `f64`: the
    // absence of an answer, not zero.
    let excluded = converted.unpriced + converted.unusable;
    let total = match converted.total {
        Some(total) if converted.priced > 0 || excluded == 0 => Value::fiat(total, code.clone()),
        _ => Value::Missing,
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
            converted.unpriced_sats.map_or(Value::Missing, Value::Sats),
            "sats of the orders with no usable rate at success_at; not in the total".to_string(),
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
    if !converted.borrowed.is_empty() {
        let sources = converted
            .borrowed
            .iter()
            .map(|(pubkey, borrowed)| format!("{} ({})", short(pubkey), borrowed.orders))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(
            "{} at another instance's rate: {sources}",
            converted.fallbacks()
        ));
    }
    if converted.unpriced > 0 {
        parts.push(format!(
            "{} with no usable rate within {}s ({} excluded)",
            converted.unpriced,
            crate::rates::MAX_AGE_SECS,
            sats(converted.unpriced_sats),
        ));
    }
    if converted.unusable > 0 {
        parts.push(format!(
            "{} unusable: the conversion is not a finite number ({} excluded)",
            converted.unusable,
            sats(converted.unusable_sats),
        ));
    }
    if converted.total.is_none() {
        parts.push("the sum of the priced orders is not a finite number".to_string());
    }
    parts.join("; ")
}

/// A sats figure for the qualification, or the fact that it does not fit.
fn sats(sats: Option<i64>) -> String {
    sats.map_or_else(
        || "more sats than a number can hold".to_string(),
        |sats| format!("{sats} sats"),
    )
}

#[cfg(test)]
mod tests;
