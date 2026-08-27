//! The market for one currency — view 5 of `docs/SPEC.md` §6.10.
//!
//! A curated selection rather than a new aggregation: which way the book
//! leans and at what premium (§6.3), how fast an order finds a taker
//! (§6.4), and which instances trade the currency at all. Everything here
//! is composed from the modules that already answer those questions, so a
//! figure cannot drift between this view and the family it came from.
//!
//! # Which orders are in a currency's market
//!
//! Two cohorts, each the one its family already uses, because a figure
//! that disagrees with the family it is quoted from is worse than a figure
//! that needs a sentence of explanation.
//!
//! Market structure (§6.3) counts the orders *standing* in the currency:
//! `fiat_code`, the currency of an order's latest version, which is what
//! `stats market --by fiat` counts a currency's orders by. Timing (§6.4)
//! counts the orders that *entered the book* in the currency:
//! `origin.fiat_code`, which is what `stats timing --by fiat` slices on,
//! because a time-to-fill is measured from the book entry and an order
//! amended from ARS to USD waited in ARS.
//!
//! The two coincide for every order that was never amended into another
//! currency, which is nearly all of them.
//!
//! # Which window a figure is counted over
//!
//! The same two populations §6.3 uses, and for the same reason: the orders
//! created in the window are the book, the orders that reached `success`
//! in it are the trades. The instance ranking by orders is a book figure
//! and the one by volume is a trades figure, so neither reports an
//! instance that has been silent all window on the strength of its
//! history.

use crate::activity::Order;
use crate::metric::{Metric, Value};
use crate::window::Window;
use crate::{timing, volume};

use super::{Market, ranking};

/// The orders standing in `fiat`'s market: the currency of their latest
/// version is `fiat`.
pub fn orders_in<'a>(orders: &'a [Order], fiat: &str) -> Vec<&'a Order> {
    orders
        .iter()
        .filter(|order| order.fiat_code == fiat)
        .collect()
}

/// The orders that entered the book in `fiat`, whatever they were amended
/// to afterwards. The cohort `stats timing --by fiat` slices out.
pub fn orders_from<'a>(orders: &'a [Order], fiat: &str) -> Vec<&'a Order> {
    orders
        .iter()
        .filter(|order| order.origin.fiat_code == fiat)
        .collect()
}

/// View 5 for `fiat` over `window`, named `market.<FIAT>.…`.
///
/// Empty of orders is not empty of rows: a currency nobody traded in the
/// window is an answer, and it reads as zeros and dashes rather than as
/// nothing at all.
pub fn report(orders: &[Order], fiat: &str, window: Window, now: i64) -> Vec<Metric> {
    let in_market: Vec<Order> = orders_in(orders, fiat).into_iter().cloned().collect();
    let from_fiat: Vec<Order> = orders_from(orders, fiat).into_iter().cloned().collect();
    let prefix = format!("market.{fiat}");
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);

    // §6.3, confined to the currency: no fiat ranking, no new currencies.
    let market: Market = super::summarise(&in_market, window);
    let mut metrics = super::metrics(&prefix, &market, Some(fiat));

    // §6.4, the half of it this view is for: how long the book takes to
    // find a taker. The rest of the lifecycle is `stats timing`.
    let timing = timing::summarise(&from_fiat, window, now);
    metrics.push(observed(
        "time_to_fill_samples",
        Value::Count(timing.time_to_fill_samples as i64),
    ));
    for (name, value) in [
        ("time_to_fill_p50", timing.time_to_fill_p50),
        ("time_to_fill_p90", timing.time_to_fill_p90),
    ] {
        metrics.push(observed(name, value.map_or(Value::Missing, Value::Seconds)));
    }
    metrics.push(observed("book_size", Value::Count(timing.book_size as i64)));

    // Who trades it: by orders put on the book in the window, and by sats
    // settled in it.
    let instance = |order: &Order| vec![order.instance.clone()];
    let book: Vec<&Order> = in_market
        .iter()
        .filter(|order| window.contains(order.created_at))
        .collect();
    let by_orders = ranking::tally(book.into_iter(), instance, |_| 1);
    let by_volume = ranking::tally(
        volume::completed(&in_market, window)
            .collect::<Vec<_>>()
            .into_iter(),
        instance,
        |order| order.amount_sats,
    );
    metrics.push(observed(
        "instances",
        Value::Count(by_orders.entries.len() as i64),
    ));
    for (name, ranking, unit) in [
        ("instances_top3_by_orders", &by_orders, ""),
        ("instances_top3_by_volume", &by_volume, " sats"),
    ] {
        metrics.push(observed(
            name,
            if ranking.is_empty() {
                Value::Missing
            } else {
                Value::Text(ranking.top3(unit))
            },
        ));
    }

    metrics
}

#[cfg(test)]
mod tests;
