//! Market structure — `docs/SPEC.md` §6.3: which way the book leans, what
//! premium it clears at, how orders are priced, and how concentrated the
//! currencies and payment methods are. `stats market [--by fiat|kind|instance]`.
//!
//! Every figure is observed: a count or a share of published orders, a
//! percentile of published premiums.
//!
//! # Which orders a figure is counted on
//!
//! Two populations, dated the way [`crate::activity`] dates them. The
//! *book* is the orders created in the window: pressure by count, the
//! market-price and range shares, the rankings by count, and the first
//! sightings. The *trades* are the orders that reached `success` in the
//! window: pressure by volume, the premiums — a premium is what a trade
//! cleared at, and an open order's is only an ask — and the rankings by
//! volume.
//!
//! Range width is relative, `(max − min) / max` — how much of the range's
//! top is play, between 0 and 1 — so that a block mixing currencies still
//! has a meaningful average.

use std::collections::BTreeSet;

use crate::activity::{self, Direction, Order};
use crate::metric::{Metric, Value};
use crate::percentile::percentile;
use crate::window::Window;

pub mod ranking;

pub use ranking::Ranking;

/// How many entries a list cell names before counting the rest.
pub const LISTED: usize = 8;

/// The ways `stats market --by` can slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Fiat,
    Kind,
    Instance,
}

/// The §6.3 figures for one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Market {
    /// Orders created in the window.
    pub orders: u64,
    /// Buys over orders created; `None` with none created.
    pub buy_orders_share: Option<f64>,
    /// Buy sats over sats completed; `None` with nothing completed.
    pub buy_volume_share: Option<f64>,
    /// Premiums in percent, over completed orders.
    pub premium_avg: Option<f64>,
    pub premium_p50: Option<f64>,
    pub premium_p50_buy: Option<f64>,
    pub premium_p50_sell: Option<f64>,
    /// `p50(sell) − p50(buy)`; `None` when either side is missing.
    pub premium_spread: Option<f64>,
    /// Orders created at market price over orders created.
    pub market_price_share: Option<f64>,
    /// Range orders over orders created.
    pub range_share: Option<f64>,
    /// Mean `(max − min) / max` over the range orders created.
    pub range_width_avg: Option<f64>,
    pub fiats_by_orders: Ranking,
    pub fiats_by_volume: Ranking,
    pub methods_by_orders: Ranking,
    pub methods_by_volume: Ranking,
    /// Currencies whose first order ever was created in the window.
    pub new_fiats: Vec<String>,
    /// Payment methods whose first order ever was created in the window.
    pub new_methods: Vec<String>,
}

/// The §6.3 figures for `orders` over `window`. `orders` is the whole
/// history in scope, not the window's: first sightings need what came
/// before.
pub fn summarise(orders: &[Order], window: Window) -> Market {
    let book: Vec<&Order> = orders
        .iter()
        .filter(|order| window.contains(order.created_at))
        .collect();
    let trades: Vec<&Order> = crate::volume::completed(orders, window).collect();

    let share = |count: usize, total: usize| (total > 0).then(|| count as f64 / total as f64);
    let buys = book
        .iter()
        .filter(|order| order.direction == Direction::Buy)
        .count();
    let sats: i64 = trades.iter().map(|order| order.amount_sats).sum();
    let buy_sats: i64 = trades
        .iter()
        .filter(|order| order.direction == Direction::Buy)
        .map(|order| order.amount_sats)
        .sum();

    let premiums = |side: Option<Direction>| -> Vec<f64> {
        trades
            .iter()
            .filter(|order| side.is_none_or(|side| order.direction == side))
            .map(|order| order.premium)
            .collect()
    };
    let all = premiums(None);
    let premium_p50_buy = percentile(&premiums(Some(Direction::Buy)), 0.5);
    let premium_p50_sell = percentile(&premiums(Some(Direction::Sell)), 0.5);

    let widths: Vec<f64> = book
        .iter()
        .filter_map(|order| order.fiat_range)
        .filter(|(_, max)| *max > 0.0)
        .map(|(min, max)| (max - min) / max)
        .collect();

    let fiat = |order: &Order| vec![order.fiat_code.clone()];
    let methods = |order: &Order| order.payment_methods.clone();
    let one = |_: &Order| 1;
    let amount = |order: &Order| order.amount_sats;

    Market {
        orders: book.len() as u64,
        buy_orders_share: share(buys, book.len()),
        buy_volume_share: (sats > 0).then(|| buy_sats as f64 / sats as f64),
        premium_avg: (!all.is_empty()).then(|| all.iter().sum::<f64>() / all.len() as f64),
        premium_p50: percentile(&all, 0.5),
        premium_p50_buy,
        premium_p50_sell,
        premium_spread: premium_p50_sell
            .zip(premium_p50_buy)
            .map(|(sell, buy)| sell - buy),
        market_price_share: share(
            book.iter().filter(|order| order.is_market_price).count(),
            book.len(),
        ),
        range_share: share(
            book.iter()
                .filter(|order| order.fiat_range.is_some())
                .count(),
            book.len(),
        ),
        range_width_avg: (!widths.is_empty())
            .then(|| widths.iter().sum::<f64>() / widths.len() as f64),
        fiats_by_orders: ranking::tally(book.iter().copied(), fiat, one),
        fiats_by_volume: ranking::tally(trades.iter().copied(), fiat, amount),
        methods_by_orders: ranking::tally(book.iter().copied(), methods, one),
        methods_by_volume: ranking::tally(trades.iter().copied(), methods, amount),
        new_fiats: first_sightings(orders, window, fiat),
        new_methods: first_sightings(orders, window, methods),
    }
}

/// The keys whose earliest order in `orders` was created in `window`.
fn first_sightings(
    orders: &[Order],
    window: Window,
    keys: impl Fn(&Order) -> Vec<String>,
) -> Vec<String> {
    let seen_before: BTreeSet<String> = orders
        .iter()
        .filter(|order| order.created_at < window.from)
        .flat_map(&keys)
        .collect();

    orders
        .iter()
        .filter(|order| window.contains(order.created_at))
        .flat_map(&keys)
        .filter(|key| !seen_before.contains(key))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The report for `stats market --by <dimension>`, or the global one when
/// `dimension` is `None`. Names follow [`crate::activity::report`]. A fiat
/// slice drops the fiat ranking and the new-currency row, which say
/// nothing about one currency.
pub fn report(orders: &[Order], window: Window, dimension: Option<Dimension>) -> Vec<Metric> {
    let Some(dimension) = dimension else {
        return metrics("market", &summarise(orders, window), true);
    };

    let (by, across_fiats) = match dimension {
        Dimension::Fiat => (activity::Dimension::Fiat, false),
        Dimension::Kind => (activity::Dimension::Kind, true),
        Dimension::Instance => (activity::Dimension::Instance, true),
    };
    activity::slice(orders, by)
        .into_iter()
        .flat_map(|(key, group)| {
            let group: Vec<Order> = group.into_iter().cloned().collect();
            metrics(
                &format!("market.{key}"),
                &summarise(&group, window),
                across_fiats,
            )
        })
        .collect()
}

/// One [`Market`] as metrics, all observed. `across_fiats` is whether
/// the block spans currencies, and so whether ranking them means anything.
pub fn metrics(prefix: &str, market: &Market, across_fiats: bool) -> Vec<Metric> {
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);
    let ratio = |value: Option<f64>| value.map_or(Value::Missing, Value::ratio);
    // A premium is published in percent; `Value::Ratio` renders as one.
    let percent = |value: Option<f64>| ratio(value.map(|premium| premium / 100.0));
    // A list cell names the first few and counts the rest: a window that
    // brings a dozen new payment methods is a fact, not a paragraph.
    let text = |items: &[String]| match items.len() {
        0 => Value::Missing,
        n if n <= LISTED => Value::Text(items.join(", ")),
        n => Value::Text(format!(
            "{}, +{} more",
            items[..LISTED].join(", "),
            n - LISTED
        )),
    };
    let ranked = |name: &str, ranking: &Ranking, unit: &str| {
        observed(
            name,
            if ranking.is_empty() {
                Value::Missing
            } else {
                Value::Text(ranking.top3(unit))
            },
        )
    };
    let concentration = |name: &str, ranking: &Ranking, what: &str| {
        let present = !ranking.is_empty();
        vec![
            observed(
                &format!("{name}_top3_{what}_share"),
                ratio(present.then_some(ranking.top3_share)),
            ),
            observed(
                &format!("{name}_hhi_{what}"),
                ratio(present.then_some(ranking.hhi)),
            ),
        ]
    };

    let mut metrics = vec![
        observed("orders", Value::Count(market.orders as i64)),
        observed("buy_orders_share", ratio(market.buy_orders_share)),
        observed("buy_volume_share", ratio(market.buy_volume_share)),
        observed("premium_avg", percent(market.premium_avg)),
        observed("premium_p50", percent(market.premium_p50)),
        observed("premium_p50_buy", percent(market.premium_p50_buy)),
        observed("premium_p50_sell", percent(market.premium_p50_sell)),
        observed("premium_spread", percent(market.premium_spread)),
        observed("market_price_share", ratio(market.market_price_share)),
        observed("range_share", ratio(market.range_share)),
        observed("range_width_avg", ratio(market.range_width_avg)),
    ];
    if across_fiats {
        metrics.push(ranked("fiat_top3_by_orders", &market.fiats_by_orders, ""));
        metrics.extend(concentration("fiat", &market.fiats_by_orders, "orders"));
        metrics.push(ranked(
            "fiat_top3_by_volume",
            &market.fiats_by_volume,
            " sats",
        ));
        metrics.extend(concentration("fiat", &market.fiats_by_volume, "volume"));
    }
    metrics.push(ranked(
        "method_top3_by_orders",
        &market.methods_by_orders,
        "",
    ));
    metrics.push(ranked(
        "method_top3_by_volume",
        &market.methods_by_volume,
        " sats",
    ));
    if across_fiats {
        metrics.push(observed("new_fiats", text(&market.new_fiats)));
    }
    metrics.push(observed("new_methods", text(&market.new_methods)));

    metrics
}

#[cfg(test)]
mod tests;
