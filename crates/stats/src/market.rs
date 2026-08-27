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
//! # Range width, twice
//!
//! A range order's width is `max − min` in its own currency, and averaging
//! those across a book that holds ARS beside EUR averages nothing. So the
//! figure every block carries is the *relative* width, `(max − min) / max`
//! — how much of the range's top is play, between 0 and 1 — which is
//! comparable across currencies. A `--by fiat` block is one currency, and
//! there the absolute width is meaningful and is reported beside it: the
//! relative form alone would say that `[10, 100]` is wider than
//! `[900, 1000]`, which in ARS it is not.
//!
//! # Payment methods
//!
//! The rankings and first sightings count the methods of an order's
//! *first* version, not of the projection. A `pm` list amended after
//! creation is not what was on the book, and a method added to an old
//! order would otherwise be dated by that order's creation and hide a
//! genuine first sighting.
//!
//! An order names several methods at once, and each ranking credits it to
//! every method it names. `method_top3_by_volume` therefore adds up to
//! more than the volume that was traded — one order of 1 361 sats offered
//! over two methods shows 1 361 against each. The sats are attributed to a
//! method, not divided between them; splitting them would invent a figure
//! nobody published. This is why the concentration rows exist only for
//! currencies, which an order names exactly one of.

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
    /// Mean `(max − min) / max` over the range orders created whose `max`
    /// is positive — the comparable form, since a block may mix
    /// currencies. A `[0, 0]` range is legal on the wire (`fa` is only
    /// checked for being non-negative) and has no relative width to take,
    /// so it is counted by [`range_share`](Self::range_share) and left out
    /// of both averages: the two denominators need not agree.
    pub range_width_avg: Option<f64>,
    /// Mean `max − min` over the same orders, in fiat. Only meaningful,
    /// and only reported, when the block is a single currency.
    pub range_width_fiat_avg: Option<f64>,
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
    // In `i128`: `amt` is admitted up to `i64::MAX`, so two such orders
    // overflow an `i64` sum — a panic in debug, a wrapped negative in
    // release, and a share silently reported as missing. The share itself
    // is a ratio, so the wider accumulator costs nothing.
    let sats: i128 = trades
        .iter()
        .map(|order| i128::from(order.amount_sats))
        .sum();
    let buy_sats: i128 = trades
        .iter()
        .filter(|order| order.direction == Direction::Buy)
        .map(|order| i128::from(order.amount_sats))
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

    let ranges: Vec<(f64, f64)> = book
        .iter()
        .filter_map(|order| order.fiat_range)
        .filter(|(_, max)| *max > 0.0)
        .collect();
    let mean = |values: Vec<f64>| {
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    };

    let fiat = |order: &Order| vec![order.fiat_code.clone()];
    let methods = |order: &Order| order.origin.payment_methods.clone();
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
        range_width_avg: mean(ranges.iter().map(|(min, max)| (max - min) / max).collect()),
        range_width_fiat_avg: mean(ranges.iter().map(|(min, max)| max - min).collect()),
        fiats_by_orders: ranking::tally(book.iter().copied(), fiat, one),
        fiats_by_volume: ranking::tally(trades.iter().copied(), fiat, amount),
        methods_by_orders: ranking::tally(book.iter().copied(), methods, one),
        methods_by_volume: ranking::tally(trades.iter().copied(), methods, amount),
        new_fiats: first_sightings(orders, window, fiat),
        new_methods: first_sightings(orders, window, methods),
    }
}

/// The keys whose earliest order in `orders` was created in `window`.
///
/// "Earliest" is earliest *in `orders`*, so what counts as new follows
/// whatever [`report`] passed in: the whole history for the global block,
/// and only the slice's own orders for a slice — see [`report`].
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
///
/// Every row of a slice is that slice's own, the first sightings
/// included: `market.buy.new_methods` names the methods whose first *buy*
/// order fell in the window, not their first order of any kind, and
/// `market.<instance>.new_fiats` names what is new *to that instance*. A
/// method a seller has offered for a year is new to the `buy` block the
/// day a buyer first names it. That is the same restriction every other
/// row of the block is under — an instance block's `orders` counts that
/// instance's orders — but it is the one row where the unqualified name
/// invites the global reading, so it is spelled out here and in the
/// README.
pub fn report(orders: &[Order], window: Window, dimension: Option<Dimension>) -> Vec<Metric> {
    let Some(dimension) = dimension else {
        return metrics("market", &summarise(orders, window), None);
    };

    let by = match dimension {
        Dimension::Fiat => activity::Dimension::Fiat,
        Dimension::Kind => activity::Dimension::Kind,
        Dimension::Instance => activity::Dimension::Instance,
    };
    activity::slice(orders, by)
        .into_iter()
        .flat_map(|(key, group)| {
            let group: Vec<Order> = group.into_iter().cloned().collect();
            // Only a fiat slice is one currency; a kind or instance block
            // still spans them.
            let fiat = matches!(dimension, Dimension::Fiat).then(|| key.clone());
            metrics(
                &format!("market.{key}"),
                &summarise(&group, window),
                fiat.as_deref(),
            )
        })
        .collect()
}

/// One [`Market`] as metrics, all observed. `fiat` is the currency the
/// block is confined to, if it is confined to one: ranking currencies
/// means nothing inside a single one, and the absolute range width means
/// nothing outside one.
pub fn metrics(prefix: &str, market: &Market, fiat: Option<&str>) -> Vec<Metric> {
    let observed = |name: &str, value: Value| Metric::observed(format!("{prefix}.{name}"), value);
    let ratio = |value: Option<f64>| value.map_or(Value::Missing, Value::ratio);
    // A premium is published in percent; `Value::Ratio` renders as one.
    let percent = |value: Option<f64>| ratio(value.map(|premium| premium / 100.0));
    // A list cell names the first few and counts the rest: a window that
    // brings a dozen new payment methods is a fact, not a paragraph.
    //
    // An empty list renders as `—`. `Value` has no empty-list form, and
    // giving these rows a `Count(0)` would make one metric a string in
    // one window and a number in the next, which no `--json` consumer can
    // type. On these two rows `—` reads as "none new in the window", not
    // as "not computed"; the README says so.
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
    if let Some((code, width)) = fiat.zip(market.range_width_fiat_avg) {
        metrics.push(observed("range_width_fiat_avg", Value::fiat(width, code)));
    }
    if fiat.is_none() {
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
    if fiat.is_none() {
        metrics.push(observed("new_fiats", text(&market.new_fiats)));
    }
    metrics.push(observed("new_methods", text(&market.new_methods)));

    metrics
}

#[cfg(test)]
mod tests;
