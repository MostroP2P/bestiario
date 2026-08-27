//! The rate lookup — `docs/SPEC.md` §5 and roadmap PR 33.
//!
//! Every converted figure in phase 3 rests on one question: what was one
//! bitcoin worth, in this currency, at this instant, as far as this
//! instance knew? [`RateBook::rate_at`] answers it from the snapshots the
//! instances published (kind 30078), and answers it *with its provenance*:
//! how old the snapshot was, and whether it came from the instance that
//! settled the order or from another one because that instance published
//! none. §5 requires every inferred figure to carry what qualifies it, and
//! a caller cannot re-derive either fact from a bare number.

use std::collections::BTreeMap;

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
    /// How long before the instant asked about the snapshot was published.
    /// Never negative: a snapshot from after the instant is not consulted.
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RateBook {
    /// Oldest first.
    snapshots: Vec<Snapshot>,
}

impl RateBook {
    pub fn new(mut snapshots: Vec<Snapshot>) -> Self {
        snapshots.sort_by(|a, b| {
            a.published_at
                .cmp(&b.published_at)
                .then_with(|| a.pubkey.cmp(&b.pubkey))
        });
        Self { snapshots }
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// The rate for `fiat` at `at_ts`, as `pubkey` knew it.
    ///
    /// The newest snapshot published *at or before* `at_ts` that carries
    /// `fiat`, from `pubkey` first; from any other instance when `pubkey`
    /// has none, and then the quote says whose it was. A snapshot from
    /// after the instant is never used: valuing a trade at a price nobody
    /// had yet seen would be a different kind of inference than the one §5
    /// describes.
    ///
    /// `None` when no instance had published a rate for `fiat` by then.
    pub fn rate_at(&self, pubkey: &str, fiat: &str, at_ts: i64) -> Option<RateQuote> {
        let candidates = || {
            self.snapshots
                .iter()
                .rev()
                .filter(|snapshot| snapshot.published_at <= at_ts)
                .filter_map(|snapshot| snapshot.rates.get(fiat).map(|rate| (snapshot, *rate)))
        };

        if let Some((snapshot, rate)) = candidates().find(|(snapshot, _)| snapshot.pubkey == pubkey)
        {
            return Some(RateQuote {
                rate,
                age_secs: at_ts - snapshot.published_at,
                source: RateSource::Instance,
            });
        }

        candidates().next().map(|(snapshot, rate)| RateQuote {
            rate,
            age_secs: at_ts - snapshot.published_at,
            source: RateSource::Fallback {
                pubkey: snapshot.pubkey.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests;
