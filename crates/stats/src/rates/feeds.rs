//! Exchange rate feeds — `docs/SPEC.md` §6.8: what each instance quotes
//! right now, how fresh its feed is, and how far the instances disagree.
//! `stats rates [--fiat F]`.
//!
//! Every figure is observed — a published price, or the age of the event
//! that carried it — and every one is about *now*, not about a window: a
//! feed is a live thing, and the question §6.8 asks is whether it is still
//! beating and what it says.
//!
//! # Freshness
//!
//! Two thresholds, each with a reason. A quote prices a trade for
//! [`MAX_AGE_SECS`], the bound §5 puts on an inferred valuation: within it
//! the feed is *fresh*. A kind 30078 event carries a NIP-40 `expiration`
//! ten minutes after it was published — both captured snapshots do — so
//! past [`DEAD_AFTER_SECS`] the instance's own event says the snapshot is
//! void and nothing has replaced it: the feed is *dead*. Between the two
//! it is *stale* — too old to price a trade, not old enough to have
//! expired. An instance that has published no snapshot at all is *silent*,
//! and one whose latest snapshot is dated after `now` is *skewed*: a clock
//! nobody shares is neither an age nor a silence.
//!
//! Every feed falls in exactly one bucket, and the buckets add up to the
//! feeds: `fresh + stale + dead + skewed` are the instances that have
//! published, `silent` the rest.
//!
//! # Disparity now
//!
//! §6.8 asks for `max/min − 1` across instances *at the same instant*, and
//! that instant is `now`: two prices an hour apart differ because the
//! market moved, which is not a disparity between instances, and a price
//! whose own event has expired is not what the feed quotes today. So only
//! the quotes that are *fresh at `now`* — the ones that could still price
//! a trade — set `low`, `high` and the ratio. Every instance quoting the
//! currency at all is still counted, `quoted_by` against `comparable`, so
//! that a currency nobody quotes live says so rather than reporting the
//! disagreement of two dead snapshots.

use std::collections::{BTreeMap, BTreeSet};

use super::MAX_AGE_SECS;
use crate::metric::{Metric, Value};

/// How long after its last snapshot a feed is called dead: the NIP-40
/// expiration a kind 30078 event carries, ten minutes past `published_at`.
/// Past it the instance itself has declared the snapshot void.
pub const DEAD_AFTER_SECS: i64 = 600;

/// What one instance quotes, as of its latest snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    /// The instance label, as in [`crate::activity::Order::instance`].
    pub instance: String,
    /// `published_at` of its latest snapshot; `None` when it has published
    /// none.
    pub published_at: Option<i64>,
    /// Price of one BTC per currency code, from that snapshot.
    pub rates: BTreeMap<String, f64>,
}

impl Feed {
    /// How long ago the feed last spoke; `None` when it never has, or when
    /// its snapshot is dated after `now` — a clock nobody shares is not an
    /// age. [`Feed::freshness`] tells the two apart.
    pub fn age(&self, now: i64) -> Option<i64> {
        self.published_at
            .filter(|published| *published <= now)
            .and_then(|published| now.checked_sub(published))
    }

    /// Where the feed stands at `now`.
    pub fn freshness(&self, now: i64) -> Freshness {
        let Some(published) = self.published_at else {
            return Freshness::Silent;
        };
        if published > now {
            return Freshness::Skewed;
        }
        // A `published_at` so far below `now` that the difference is not a
        // number is as dead as a snapshot gets; it is never a young one.
        match now.checked_sub(published).unwrap_or(i64::MAX) {
            age if age <= MAX_AGE_SECS => Freshness::Fresh,
            age if age <= DEAD_AFTER_SECS => Freshness::Stale,
            _ => Freshness::Dead,
        }
    }

    /// What the feed quotes for `fiat`, if anything. A price that is not a
    /// finite positive number is not a price, whoever stored it.
    pub fn rate(&self, fiat: &str) -> Option<f64> {
        self.rates
            .get(fiat)
            .copied()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
    }

    /// The rate this feed can still price a trade with at `now`: its quote
    /// for `fiat`, and only while the feed is fresh.
    fn live_rate(&self, fiat: &str, now: i64) -> Option<f64> {
        (self.freshness(now) == Freshness::Fresh)
            .then(|| self.rate(fiat))
            .flatten()
    }
}

/// Where a feed stands — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Within the valuation bound: it can still price a trade.
    Fresh,
    /// Past the bound, not past the event's own expiry.
    Stale,
    /// Past the expiry its own event carried, with nothing since.
    Dead,
    /// Nothing published, ever.
    Silent,
    /// Its latest snapshot is dated after `now`: the instance published,
    /// but by a clock this machine does not share, so the snapshot has no
    /// age and prices nothing.
    Skewed,
}

impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Dead => "dead",
            Self::Silent => "silent",
            Self::Skewed => "skewed",
        }
    }
}

/// The state of the feeds as a whole at `now`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// Instances that have published at least one snapshot — every bucket
    /// but `silent`.
    pub feeds: u64,
    pub fresh: u64,
    pub stale: u64,
    pub dead: u64,
    pub silent: u64,
    pub skewed: u64,
    /// Distinct currency codes quoted across every feed.
    pub currencies: u64,
}

/// The §6.8 counts for `feeds` at `now`.
pub fn summarise(feeds: &[Feed], now: i64) -> Summary {
    let mut summary = Summary::default();
    let mut codes: BTreeSet<&str> = BTreeSet::new();

    for feed in feeds {
        match feed.freshness(now) {
            Freshness::Fresh => summary.fresh += 1,
            Freshness::Stale => summary.stale += 1,
            Freshness::Dead => summary.dead += 1,
            Freshness::Skewed => summary.skewed += 1,
            Freshness::Silent => summary.silent += 1,
        }
        if feed.published_at.is_some() {
            summary.feeds += 1;
        }
        codes.extend(feed.rates.keys().map(String::as_str));
    }
    summary.currencies = codes.len() as u64;

    summary
}

/// How far the instances disagree about one currency, now. A single
/// comparable quote is no disagreement: `ratio` is then `None` and the
/// report says nothing rather than zero.
#[derive(Debug, Clone, PartialEq)]
pub struct Disparity {
    /// Instances quoting the currency at all, whatever the age of their
    /// snapshot.
    pub quoted_by: u64,
    /// Those whose quote is still fresh at `now`.
    pub comparable: u64,
    /// The cheapest and dearest of *those*; `None` when none is fresh.
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// `high / low − 1`; `None` unless two quotes are comparable.
    pub ratio: Option<f64>,
}

/// The disagreement over `fiat` at `now`, or `None` when no feed quotes it
/// at all.
///
/// Only the quotes that are fresh at `now` enter `low`, `high` and the
/// ratio; see the module docs.
pub fn disparity(feeds: &[Feed], fiat: &str, now: i64) -> Option<Disparity> {
    let quoted_by = feeds
        .iter()
        .filter(|feed| feed.rate(fiat).is_some())
        .count();
    if quoted_by == 0 {
        return None;
    }

    let comparable: Vec<f64> = feeds
        .iter()
        .filter_map(|feed| feed.live_rate(fiat, now))
        .collect();
    let low = comparable.iter().copied().reduce(f64::min);
    let high = comparable.iter().copied().reduce(f64::max);

    Some(Disparity {
        quoted_by: quoted_by as u64,
        comparable: comparable.len() as u64,
        low,
        high,
        // One voice disagrees with nobody: a disparity needs two quotes
        // standing at the same instant, and without them there is no
        // answer rather than a zero.
        ratio: (comparable.len() > 1)
            .then(|| Some(high? / low? - 1.0))
            .flatten(),
    })
}

/// The report for `stats rates`, and for `--fiat <FIAT>` the block for
/// that currency after the summary. Names follow
/// [`crate::activity::report`]: the instance label is the key.
pub fn report(feeds: &[Feed], fiat: Option<&str>, now: i64) -> Vec<Metric> {
    let observed = |name: String, value: Value| Metric::observed(format!("rates.{name}"), value);
    let count = |name: String, value: u64| observed(name, Value::Count(value as i64));
    let summary = summarise(feeds, now);

    // Every bucket is named, so that the statuses below reconcile with the
    // block above them and a JSON consumer never has to infer a count from
    // the per-instance rows.
    let mut metrics = vec![
        count("feeds".to_string(), summary.feeds),
        count("fresh".to_string(), summary.fresh),
        count("stale".to_string(), summary.stale),
        count("dead".to_string(), summary.dead),
        count("silent".to_string(), summary.silent),
        count("skewed".to_string(), summary.skewed),
        count("currencies".to_string(), summary.currencies),
    ];

    if let Some(fiat) = fiat {
        let quoted = disparity(feeds, fiat, now);
        let amount =
            |value: Option<f64>| value.map_or(Value::Missing, |amount| Value::fiat(amount, fiat));
        metrics.push(count(
            format!("{fiat}.quoted_by"),
            quoted.as_ref().map_or(0, |quoted| quoted.quoted_by),
        ));
        metrics.push(count(
            format!("{fiat}.comparable"),
            quoted.as_ref().map_or(0, |quoted| quoted.comparable),
        ));
        metrics.push(observed(
            format!("{fiat}.low"),
            amount(quoted.as_ref().and_then(|quoted| quoted.low)),
        ));
        metrics.push(observed(
            format!("{fiat}.high"),
            amount(quoted.as_ref().and_then(|quoted| quoted.high)),
        ));
        metrics.push(observed(
            format!("{fiat}.disparity"),
            quoted
                .as_ref()
                .and_then(|quoted| quoted.ratio)
                .map_or(Value::Missing, Value::ratio),
        ));
        for feed in sorted(feeds) {
            if let Some(rate) = feed.rate(fiat) {
                metrics.push(observed(
                    format!("{fiat}.{}", feed.instance),
                    Value::fiat(rate, fiat),
                ));
            }
        }
    }

    for feed in sorted(feeds) {
        let instance = &feed.instance;
        metrics.push(observed(
            format!("{instance}.age"),
            feed.age(now).map_or(Value::Missing, Value::Seconds),
        ));
        metrics.push(observed(
            format!("{instance}.status"),
            Value::Text(feed.freshness(now).as_str().to_string()),
        ));
        metrics.push(count(
            format!("{instance}.currencies"),
            feed.rates.len() as u64,
        ));
    }

    metrics
}

/// The feeds by instance label, the order every report lists instances in.
fn sorted(feeds: &[Feed]) -> Vec<&Feed> {
    let mut sorted: Vec<&Feed> = feeds.iter().collect();
    sorted.sort_by(|a, b| a.instance.cmp(&b.instance));
    sorted
}

#[cfg(test)]
mod tests;
