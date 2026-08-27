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

/// The NIP-65 marker for a relay the instance only reads from.
const READ_ONLY: &str = "read";

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
        // No marker means read and write both (NIP-65).
        if values.get(1).is_some_and(|marker| marker == READ_ONLY) {
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

#[cfg(test)]
mod tests;
