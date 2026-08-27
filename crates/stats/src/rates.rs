//! The rate lookup — `docs/SPEC.md` §5 and roadmap PR 33.
//!
//! Every converted figure in phase 3 rests on one question: what was one
//! bitcoin worth, in this currency, at this instant, as far as this
//! instance knew? [`RateBook::rate_at`] answers it from the snapshots the
//! instances published (kind 30078), and answers it *with its provenance*:
//! how old the snapshot was, and whether it came from the instance that
//! settled the order or from another one because that instance had none.
//! §5 requires every inferred figure to carry what qualifies it, and a
//! caller cannot re-derive either fact from a bare number.
//!
//! §5 also bounds the inference: a rate is usable for up to five minutes.
//! The bound is enforced here rather than left to callers — a quote that
//! exists is a quote that is valid — so a feed outage or an ingestion gap
//! yields no rate, never a stale one dressed as a price.

use std::collections::BTreeMap;

/// How old a snapshot may be and still price an order (§5).
pub const MAX_AGE_SECS: i64 = 300;

/// One rate snapshot as the lookup sees it: who published it, when, and
/// the price of one BTC per currency code.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub pubkey: String,
    pub published_at: i64,
    pub rates: BTreeMap<String, f64>,
}

/// Where a quote came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateSource {
    /// The instance asked about published it.
    Instance,
    /// That instance had no usable snapshot; this other one did.
    Fallback { pubkey: String },
}

/// A rate, and what qualifies it.
#[derive(Debug, Clone, PartialEq)]
pub struct RateQuote {
    /// Price of one BTC in the currency asked for.
    pub rate: f64,
    /// How long before the instant asked about the snapshot was published:
    /// between `0` and [`MAX_AGE_SECS`]. Never negative — a snapshot from
    /// after the instant is not consulted — and never above the bound.
    pub age_secs: i64,
    pub source: RateSource,
}

impl RateQuote {
    /// Sats to fiat at this rate.
    pub fn convert_sats(&self, sats: i64) -> f64 {
        sats as f64 / 100_000_000.0 * self.rate
    }
}

/// Every snapshot known, ready to be asked.
///
/// Indexed for the way it is asked: once per converted order, for one
/// instance at one instant. Snapshots sit in one vector ordered by
/// `published_at`, with the positions of each instance's own in a map, so
/// a lookup is a binary search to the instant and a walk back across the
/// five minutes that can qualify — not a scan of the history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RateBook {
    /// Oldest first.
    snapshots: Vec<Snapshot>,
    /// Positions into `snapshots`, ascending, per publishing instance.
    by_pubkey: BTreeMap<String, Vec<usize>>,
    /// Every position, ascending: what the fallback searches.
    everyone: Vec<usize>,
}

impl RateBook {
    pub fn new(mut snapshots: Vec<Snapshot>) -> Self {
        snapshots.sort_by(|a, b| {
            a.published_at
                .cmp(&b.published_at)
                .then_with(|| a.pubkey.cmp(&b.pubkey))
        });
        let mut by_pubkey: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (position, snapshot) in snapshots.iter().enumerate() {
            by_pubkey
                .entry(snapshot.pubkey.clone())
                .or_default()
                .push(position);
        }
        Self {
            everyone: (0..snapshots.len()).collect(),
            snapshots,
            by_pubkey,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// The rate for `fiat` at `at_ts`, as `pubkey` knew it.
    ///
    /// The newest snapshot published *at or before* `at_ts` and no more
    /// than [`MAX_AGE_SECS`] before it that carries `fiat`, from `pubkey`
    /// first; from any other instance when `pubkey` has none that
    /// qualifies, and then the quote says whose it was. A snapshot from
    /// after the instant is never used: valuing a trade at a price nobody
    /// had yet seen would be a different kind of inference than the one
    /// §5 describes. Nor is one older than the bound: that is the price of
    /// some other moment.
    ///
    /// `None` when no instance had a usable rate for `fiat` at that instant.
    pub fn rate_at(&self, pubkey: &str, fiat: &str, at_ts: i64) -> Option<RateQuote> {
        let own = self
            .by_pubkey
            .get(pubkey)
            .and_then(|positions| self.newest(positions, fiat, at_ts));
        if let Some((snapshot, rate)) = own {
            return Some(RateQuote {
                rate,
                age_secs: at_ts - snapshot.published_at,
                source: RateSource::Instance,
            });
        }

        // `everyone` is this index, built once: rebuilding it here made an
        // unquoted currency cost one allocation over the whole archive per
        // order asked about.
        self.newest(&self.everyone, fiat, at_ts)
            .map(|(snapshot, rate)| RateQuote {
                rate,
                age_secs: at_ts - snapshot.published_at,
                source: RateSource::Fallback {
                    pubkey: snapshot.pubkey.clone(),
                },
            })
    }

    /// Among `positions` (ascending by time), the newest snapshot within
    /// the bound before `at_ts` that quotes `fiat`.
    fn newest(&self, positions: &[usize], fiat: &str, at_ts: i64) -> Option<(&Snapshot, f64)> {
        let end = positions.partition_point(|&i| self.snapshots[i].published_at <= at_ts);
        positions[..end]
            .iter()
            .rev()
            .map(|&i| &self.snapshots[i])
            .take_while(|snapshot| at_ts - snapshot.published_at <= MAX_AGE_SECS)
            .find_map(|snapshot| snapshot.rates.get(fiat).map(|rate| (snapshot, *rate)))
    }
}

pub mod feeds;

#[cfg(test)]
mod tests;
