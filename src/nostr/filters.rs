//! Construction of `Filter` values per kind, author set and time window.
//!
//! Pure functions: they take configuration and a time range and return a
//! filter. No network access, so they are unit-testable on their own.
//!
//! # The one conversion worth naming
//!
//! [`Range`] is half-open — `from <= t < until` — so that consecutive
//! reporting windows tile without counting a boundary event twice. A Nostr
//! filter is inclusive at *both* ends. Handing `until` straight to a relay
//! would therefore fetch one second more than the window asked for, and the
//! event that lands exactly on the boundary would be pulled by two adjacent
//! backfill windows. [`window`] subtracts that second, in one place, so no
//! caller has to remember it.

use nostr_sdk::prelude::{Filter, Kind, PublicKey, Timestamp};

use crate::commands::range::Range;
use crate::ingest::parse::{dev_fee, dispute, info, order, rates};

/// The kinds the pipeline can parse today (`docs/SPEC.md` §2.1–§2.5).
///
/// Kind 10002 is in the spec but has no parser yet, so it is deliberately
/// absent: subscribing to events nothing can read would spend a relay's
/// bandwidth to fill the rejected counter. [`for_kind`] takes any kind, so it
/// costs one line when its parser lands.
pub const INDEXED_KINDS: [u16; 5] = [
    order::KIND,
    dev_fee::KIND,
    dispute::KIND,
    info::KIND,
    rates::KIND,
];

/// A filter for one kind, optionally narrowed to `authors` and to `range`.
///
/// An empty `authors` slice means *any* author, which is what
/// `accept_unknown_instances` asks for: the platform filter of SPEC §8.1
/// step 4 then decides what is really a Mostro event. Listing no authors is
/// not the same as listing none of them — a filter with an empty author list
/// would match nothing at all — so the field is left off entirely.
pub fn for_kind(
    kind: u16,
    authors: &[PublicKey],
    range: Option<Range>,
    limit: Option<usize>,
) -> Filter {
    let mut filter = Filter::new().kind(Kind::from_u16(kind));

    if !authors.is_empty() {
        filter = filter.authors(authors.iter().copied());
    }

    if let Some(range) = range {
        let (since, until) = window(range);
        if let Some(since) = since {
            filter = filter.since(since);
        }
        if let Some(until) = until {
            filter = filter.until(until);
        }
    }

    match limit {
        Some(limit) => filter.limit(limit),
        None => filter,
    }
}

/// One filter per kind in [`INDEXED_KINDS`], in that order.
///
/// One filter per kind rather than a single filter listing all four, because
/// the resume cursor of `repo::sync_state` is per `(relay, kind)`: a shared
/// filter would have to use the oldest cursor of the four and re-read
/// everything the other three had already covered.
pub fn per_kind(authors: &[PublicKey], range: Option<Range>, limit: Option<usize>) -> Vec<Filter> {
    INDEXED_KINDS
        .iter()
        .map(|&kind| for_kind(kind, authors, range, limit))
        .collect()
}

/// The inclusive `(since, until)` pair covering the half-open `range`, with an
/// open end reported as `None` rather than as a number.
///
/// `until` is the last second *inside* the window, not the first one after it.
///
/// [`Range::unbounded`] represents "all of recorded time" with the sentinels
/// `0` and `i64::MAX`. Passed through arithmetically those would put
/// `until = 9223372036854775806` on the wire — a timestamp no relay can mean
/// anything by, and one that says the opposite of what it is trying to say.
/// An open end is an *absent* field in a Nostr filter, so that is what it
/// becomes here.
fn window(range: Range) -> (Option<Timestamp>, Option<Timestamp>) {
    let since = (range.from() > 0).then(|| Timestamp::from_secs(range.from() as u64));
    let until =
        (range.until() < i64::MAX).then(|| Timestamp::from_secs((range.until() - 1).max(0) as u64));

    (since, until)
}

#[cfg(test)]
mod tests;
