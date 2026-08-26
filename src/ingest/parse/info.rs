//! Parser for kind 38385 — instance info (`docs/SPEC.md` §2.4).
//!
//! This is the instance describing itself: the fee it charges, the order
//! bounds it accepts, the currencies it lists, the version it runs. Every
//! version is kept, because `fee` changes over time and phase 3 needs the
//! value *in force at the time of an order*, not the newest one.
//!
//! Almost every field is optional. An info event is a self-description that
//! grows tag by tag across mostrod releases — the capture behind
//! `tests/fixtures` already shows one instance publishing no
//! `protocol_version` and another publishing no bond policy — so refusing an
//! event over a field it never promised would cost bestiario the whole
//! instance rather than one column of it.
//!
//! **`dev_fee_percentage` is not published here.** It is the one number
//! bestiario cannot observe and has to assume from configuration
//! (`docs/SPEC.md` §5).

use nostr_sdk::prelude::Event;

use super::{ParseError, expect_kind, number, optional, required};

/// The kind this parser accepts.
pub const KIND: u16 = 38385;

/// One published self-description of an instance — one 38385 event, parsed.
///
/// Mirrors `instance_info` in `docs/SPEC.md` §4. `fiat_currencies` keeps the
/// csv exactly as published rather than a parsed list: it is reported back as
/// a list of currencies an instance accepts, and re-splitting it at read time
/// costs nothing next to storing an interpretation that may not round-trip.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceInfo {
    pub event_id: String,
    pub pubkey: String,
    pub created_at: i64,
    /// The per-side fee as a fraction, e.g. `0.006`.
    pub fee: Option<f64>,
    pub max_order_amount: Option<i64>,
    pub min_order_amount: Option<i64>,
    pub fiat_currencies: Option<String>,
    pub mostro_version: Option<String>,
    pub protocol_version: Option<String>,
    pub ln_networks: Option<String>,
    pub bond_enabled: Option<bool>,
}

/// Turn a 38385 event into an [`InstanceInfo`].
pub fn parse(event: &Event) -> Result<InstanceInfo, ParseError> {
    expect_kind(event, KIND)?;

    // The `d` tag of an addressable 38385 is the instance's own pubkey, so a
    // mismatch means the event does not describe its publisher and nothing
    // downstream could say whose fee it is.
    let identifier = required(event, "d")?;
    let pubkey = event.pubkey.to_hex();
    if identifier != pubkey {
        return Err(ParseError::UnknownValue {
            tag: "d",
            value: identifier,
            expected: "the publishing instance's own pubkey",
        });
    }

    Ok(InstanceInfo {
        event_id: event.id.to_hex(),
        pubkey,
        created_at: event.created_at.as_secs() as i64,
        fee: optional_number(event, "fee", "a fraction, e.g. `0.006`")?,
        max_order_amount: optional_number(event, "max_order_amount", "an amount in sats")?,
        min_order_amount: optional_number(event, "min_order_amount", "an amount in sats")?,
        fiat_currencies: optional(event, "fiat_currencies_accepted")?,
        mostro_version: optional(event, "mostro_version")?,
        protocol_version: optional(event, "protocol_version")?,
        ln_networks: optional(event, "lnd_networks")?,
        bond_enabled: optional_bool(event, "bond_enabled")?,
    })
}

/// A tag that may be absent, but must be a number when it is present: an
/// unreadable fee is worse than an absent one, because phase 3 would divide
/// by it.
fn optional_number<T: std::str::FromStr>(
    event: &Event,
    tag: &'static str,
    expected: &'static str,
) -> Result<Option<T>, ParseError> {
    optional(event, tag)?
        .map(|value| number(tag, &value, expected))
        .transpose()
}

/// Booleans arrive as the strings mostrod prints, and nothing else counts as
/// false — an unrecognised value is an error rather than a silent `false`.
fn optional_bool(event: &Event, tag: &'static str) -> Result<Option<bool>, ParseError> {
    let Some(value) = optional(event, tag)? else {
        return Ok(None);
    };

    match value.as_str() {
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        _ => Err(ParseError::UnknownValue {
            tag,
            value,
            expected: "`true` or `false`",
        }),
    }
}

#[cfg(test)]
mod tests;
