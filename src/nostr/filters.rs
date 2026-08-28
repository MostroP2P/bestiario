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
use crate::ingest::parse::{dev_fee, dispute, info, order, rates, relay_list};
use crate::ingest::pipeline::UNTAGGED_KINDS;

/// The kinds the pipeline can parse (`docs/SPEC.md` §2.1–§2.6).
///
/// The order matters for the two kinds that carry no `y` tag: rates and
/// relay lists are vouched for by their publisher having already been seen
/// as an instance, so they are walked last, after the tagged kinds have
/// archived who that is.
pub const INDEXED_KINDS: [u16; 6] = [
    order::KIND,
    dev_fee::KIND,
    dispute::KIND,
    info::KIND,
    rates::KIND,
    relay_list::KIND,
];

/// The kinds a relay holds exactly one of per publisher.
///
/// An instance publishes one profile, one rate sheet and one relay list,
/// each replacing the last, so a relay has one copy of each however long it
/// has been running. The rest accumulate: an order and a dispute are
/// addressable per trade and a dev fee is not replaceable at all, so a relay
/// holds as many as its retention allows and the oldest one says how far
/// back that reaches.
///
/// The distinction matters to [`crate::db::repo::events::earliest_created_at`]
/// alone, and only because the two are opposite there. For an accumulating
/// kind the earliest event stored is evidence of reach. For one of these it
/// is evidence of nothing but when this archive started: a backfill asking
/// for January's rates is handed today's, and reading that as a coverage
/// floor would move the floor of every report to the day the indexer was
/// deployed — publishing an archive that holds the year as one that holds
/// today.
pub const SINGLE_COPY_KINDS: [u16; 3] = [info::KIND, rates::KIND, relay_list::KIND];

/// A filter for one kind, narrowed to `range` and to whichever author set
/// that kind is entitled to; `None` when this run must not ask for it at
/// all.
///
/// # Which authors
///
/// A tagged kind gets `authors`, where an empty slice means *any* author —
/// what `accept_unknown_instances` asks for, with the platform filter of
/// SPEC §8.1 step 4 deciding afterwards what is really a Mostro event.
/// Listing no authors is not the same as listing none of them — a filter
/// with an empty author list would match nothing at all — so the field is
/// left off entirely.
///
/// An untagged kind gets `vouched`, whatever `authors` says. Step 4b takes
/// a kind 30078 or 10002 only from a publisher already listed or already
/// seen publishing a `y = mostro` event, so every other answer would be
/// downloaded, verified and thrown away. For 10002 that is not merely
/// wasteful: it is the kind *every* Nostr user publishes, and asking for it
/// of no author in particular turns relay discovery into a crawl of the
/// network's whole NIP-65 index. With nobody vouched yet there is no such
/// request to make, and the kind is skipped rather than asked of everyone.
pub fn for_kind(
    kind: u16,
    authors: &[PublicKey],
    vouched: &[PublicKey],
    range: Option<Range>,
    limit: Option<usize>,
) -> Option<Filter> {
    let authors = if UNTAGGED_KINDS.contains(&kind) {
        if vouched.is_empty() {
            return None;
        }
        vouched
    } else {
        authors
    };

    let mut filter = Filter::new().kind(Kind::from_u16(kind));

    // Kind 30078 is NIP-78's generic application-data kind, shared by every
    // application storing under it, so the kind alone does not describe a
    // rate snapshot — the `d` does. Without this the relay would send every
    // unrelated 30078 address for the pipeline to verify, archive as a
    // rejection and never advance a cursor over.
    if kind == rates::KIND {
        filter = filter.identifier(rates::IDENTIFIER);
    }

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

    Some(match limit {
        Some(limit) => filter.limit(limit),
        None => filter,
    })
}

/// One filter per kind in [`INDEXED_KINDS`], in that order, leaving out the
/// kinds this run has nobody to ask about.
///
/// One filter per kind rather than a single filter listing all four, because
/// the resume cursor of `repo::sync_state` is per `(relay, kind)`: a shared
/// filter would have to use the oldest cursor of the four and re-read
/// everything the other three had already covered.
pub fn per_kind(
    authors: &[PublicKey],
    vouched: &[PublicKey],
    range: Option<Range>,
    limit: Option<usize>,
) -> Vec<Filter> {
    INDEXED_KINDS
        .iter()
        .filter_map(|&kind| for_kind(kind, authors, vouched, range, limit))
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
