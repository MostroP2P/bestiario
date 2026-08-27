//! Parser for kind 10002 — NIP-65 relay lists (`docs/SPEC.md` §2.6).
//!
//! An instance publishes where it reads and where it writes. bestiario cares
//! about the second: an event of that instance can only be fetched from a
//! relay it publishes to, so those are the relays worth adding to the
//! connection set when `discover_relays` is on. A relay it only *reads* from
//! holds nothing of its own, and dialling it would spend a connection to
//! index nothing.
//!
//! # Why a bad entry is dropped rather than refused
//!
//! Every other parser here rejects the whole event when a tag is wrong,
//! because the event *is* the datum. A relay list is a set of independent
//! claims: one unparseable URL says nothing about the other five, and
//! throwing them away would cost an instance's whole list over a typo.
//! What cannot be dialled is dropped, and what can is kept.

use std::collections::BTreeSet;

use nostr_sdk::prelude::{Event, RelayUrl};

use super::{ParseError, expect_kind, repeated_tag_values};

/// The kind this parser accepts.
pub const KIND: u16 = 10002;

/// The NIP-65 marker for a relay the instance publishes to. The other two
/// cases the NIP defines are an absent marker — read and write both — and
/// `read`, which is not a place the instance's own events land.
const WRITE: &str = "write";

/// One relay list — one 10002 event, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayList {
    pub event_id: String,
    pub pubkey: String,
    pub created_at: i64,
    /// The relays the instance publishes to, normalised and deduplicated,
    /// in the order published.
    pub relays: Vec<String>,
}

/// Turn a 10002 event into a [`RelayList`].
pub fn parse(event: &Event) -> Result<RelayList, ParseError> {
    expect_kind(event, KIND)?;

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut relays = Vec::new();
    for values in repeated_tag_values(event, "r") {
        let Some(url) = values.first() else {
            continue;
        };
        // NIP-65 defines three cases and no more: no marker at all, which
        // means read and write both; `write`; and `read`. A marker that is
        // none of them is not a write claim — it is an entry nobody can
        // read the meaning of, and it is dropped like a URL nobody can
        // dial rather than dialled on the strength of not saying `read`.
        if !writes_to(values.get(1)) {
            continue;
        }
        let Ok(url) = RelayUrl::parse(url) else {
            continue;
        };
        // `wss://host` and `wss://host/` are the same relay; the client
        // dials one of them, so the table and the connection set had
        // better hold one of them too.
        let url = url.as_str_without_trailing_slash().to_string();
        if seen.insert(url.clone()) {
            relays.push(url);
        }
    }

    Ok(RelayList {
        event_id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs() as i64,
        relays,
    })
}

/// Whether an `r` tag's marker claims the instance publishes there.
///
/// Only the two NIP-65 spellings that carry a write claim do: the marker
/// left off, and [`WRITE`]. Everything else — `read`, a typo, a marker from
/// some other convention — says nothing bestiario can act on.
fn writes_to(marker: Option<&String>) -> bool {
    marker.is_none_or(|marker| marker == WRITE)
}

#[cfg(test)]
mod tests;
