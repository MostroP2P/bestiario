//! Parser for kind 38386 — disputes (`docs/SPEC.md` §2.3).
//!
//! A dispute event carries **no `order-id`**, so the dispute→order linkage is
//! not observable from the outside and nothing here tries to reconstruct it;
//! that linkage is what `docs/SPEC.md` §6.9 lists as unmeasurable. The
//! aggregate dispute rate of §6.7 — # disputes opened / # orders that left
//! `pending`, per instance — is still measurable, because it divides two
//! counts and never needs to pair a dispute with its order.
//!
//! Note the two timestamps. The event's own `created_at` is when *this
//! version* was published; the `created_at` **tag** is when the dispute was
//! opened. Reading one for the other would date every dispute to the moment
//! of its last state change.

use nostr_sdk::prelude::Event;

use super::{ParseError, expect_kind, number, optional, required};

/// The kind this parser accepts.
pub const KIND: u16 = 38386;

/// Who raised the dispute, as published in the `initiator` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Initiator {
    Buyer,
    Seller,
}

/// The dispute statuses mostrod publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Initiated,
    InProgress,
    SellerRefunded,
    Settled,
    Released,
}

/// One published version of a dispute — one 38386 event, parsed.
///
/// Mirrors `dispute_versions` in `docs/SPEC.md` §4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeVersion {
    pub event_id: String,
    pub dispute_id: String,
    pub pubkey: String,
    /// When this version was published.
    pub created_at: i64,
    pub status: Status,
    pub initiator: Option<Initiator>,
    /// When the dispute was opened — the `created_at` *tag*.
    pub opened_at: Option<i64>,
}

impl Initiator {
    /// The wire form, as it appears in the `initiator` tag.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buyer => "buyer",
            Self::Seller => "seller",
        }
    }

    fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "buyer" => Ok(Self::Buyer),
            "seller" => Ok(Self::Seller),
            _ => Err(ParseError::UnknownValue {
                tag: "initiator",
                value: value.to_string(),
                expected: "`buyer` or `seller`",
            }),
        }
    }
}

impl Status {
    /// The wire form, as it appears in the `s` tag and in the `status` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initiated => "initiated",
            Self::InProgress => "in-progress",
            Self::SellerRefunded => "seller-refunded",
            Self::Settled => "settled",
            Self::Released => "released",
        }
    }

    fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "initiated" => Ok(Self::Initiated),
            "in-progress" => Ok(Self::InProgress),
            "seller-refunded" => Ok(Self::SellerRefunded),
            "settled" => Ok(Self::Settled),
            "released" => Ok(Self::Released),
            _ => Err(ParseError::UnknownValue {
                tag: "s",
                value: value.to_string(),
                expected: "`initiated`, `in-progress`, `seller-refunded`, `settled` or `released`",
            }),
        }
    }
}

/// Turn a 38386 event into a [`DisputeVersion`].
pub fn parse(event: &Event) -> Result<DisputeVersion, ParseError> {
    expect_kind(event, KIND)?;

    let initiator = optional(event, "initiator")?
        .map(|value| Initiator::parse(&value))
        .transpose()?;
    let opened_at = optional(event, "created_at")?
        .map(|value| number("created_at", &value, "a unix timestamp"))
        .transpose()?;

    Ok(DisputeVersion {
        event_id: event.id.to_hex(),
        dispute_id: required(event, "d")?,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs() as i64,
        status: Status::parse(&required(event, "s")?)?,
        initiator,
        opened_at,
    })
}

#[cfg(test)]
mod tests;
