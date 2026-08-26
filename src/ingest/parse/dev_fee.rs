//! Parser for kind 8383 — dev-fee payments (`docs/SPEC.md` §2.2).
//!
//! An instance publishes one of these when it actually pays the development
//! fee for a settled order, so a dev fee is the only *proof* bestiario has
//! that a trade completed and moved money. It is also the longest-lived
//! record on the relays — a NIP-40 expiration of a year, against the order's
//! own — which makes it the backbone of historical backfill.
//!
//! Two consequences for this parser:
//!
//! - `order-id` is required but is **not** a foreign key. A dev fee routinely
//!   arrives for an order whose 38383 has already expired off the relays, and
//!   dropping it would silently shrink the settled-volume figures.
//! - `destination` and `network` are optional. They describe where the
//!   payment went, not that it happened; refusing the event over them would
//!   trade a whole settlement for a missing label.

use nostr_sdk::prelude::Event;

use super::{ParseError, expect_kind, number, optional, optional_network, required};
use crate::network::Network;

/// The kind this parser accepts.
pub const KIND: u16 = 8383;

/// One dev-fee payment — one 8383 event, parsed.
///
/// Mirrors `dev_fees` in `docs/SPEC.md` §4, minus `is_duplicate`: whether this
/// is the second fee seen for the same order is a question about the database,
/// not about the event, and it is answered by the repository (PR 14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevFee {
    pub event_id: String,
    pub pubkey: String,
    pub order_id: String,
    pub created_at: i64,
    pub amount_sats: i64,
    pub payment_hash: String,
    pub destination: Option<String>,
    pub network: Option<Network>,
}

/// Turn an 8383 event into a [`DevFee`].
pub fn parse(event: &Event) -> Result<DevFee, ParseError> {
    expect_kind(event, KIND)?;

    Ok(DevFee {
        event_id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        order_id: required(event, "order-id")?,
        created_at: event.created_at.as_secs() as i64,
        amount_sats: number("amount", &required(event, "amount")?, "an amount in sats")?,
        payment_hash: required(event, "hash")?,
        destination: optional(event, "destination")?,
        network: optional_network(event)?,
    })
}

#[cfg(test)]
mod tests;
