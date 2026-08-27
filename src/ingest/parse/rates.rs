//! Parser for kind 30078 — exchange rate snapshots (`docs/SPEC.md` §2.5).
//!
//! An instance publishes, about once an hour, the price of one bitcoin in
//! every currency its rate source knows: `{"BTC": {"USD": 50000.0, …}}` in
//! the content, with the source named in a tag. Every snapshot is kept, so
//! that a completed order can be valued at the rate *in force when it
//! completed* (phase 3) rather than at today's.
//!
//! # Why every snapshot is stored locally
//!
//! Kind 30078 is addressable (NIP-01): a relay keeps only the newest event
//! per `(pubkey, kind, d)`, and `d` is the fixed `mostro-rates`, so a relay
//! holds exactly one snapshot per instance — the last one. The history that
//! phase 3 values orders against therefore only exists in the `rates` table
//! this parser feeds: what is not captured while it is the current event is
//! gone from that relay. That is what makes `repo::rates` an archive rather
//! than a cache, and why a snapshot is never overwritten or pruned.
//!
//! The content is validated as strictly as a tag would be: a rate that is
//! not a finite, positive number is not a rate, and one that slipped through
//! would multiply every converted figure by nonsense.

use std::collections::BTreeMap;

use nostr_sdk::prelude::Event;

use super::{ParseError, expect_kind, non_negative, number, optional, required};

/// The kind this parser accepts.
pub const KIND: u16 = 30078;

/// The `d` tag a Mostro rate snapshot carries. Kind 30078 is a generic
/// application-data kind (NIP-78); this identifier is what makes an event
/// of it a rate snapshot rather than anything else.
pub const IDENTIFIER: &str = "mostro-rates";

/// One rate snapshot — one 30078 event, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct RateSnapshot {
    pub event_id: String,
    pub pubkey: String,
    /// When the rates were fetched, from the `published_at` tag; the event's
    /// own `created_at` when the tag is absent. The two are the same second
    /// in every captured event, and the tag is the instance's own claim.
    pub published_at: i64,
    /// The rate provider, e.g. `yadio`.
    pub source: Option<String>,
    /// Price of one BTC per currency code, as published.
    pub rates: BTreeMap<String, f64>,
}

/// Turn a 30078 event into a [`RateSnapshot`].
pub fn parse(event: &Event) -> Result<RateSnapshot, ParseError> {
    expect_kind(event, KIND)?;

    let identifier = required(event, "d")?;
    if identifier != IDENTIFIER {
        return Err(ParseError::UnknownValue {
            tag: "d",
            value: identifier,
            expected: "`mostro-rates`",
        });
    }

    let created_at = event.created_at.as_secs() as i64;
    let published_at = match optional(event, "published_at")? {
        Some(value) => non_negative(
            "published_at",
            number::<i64>("published_at", &value, "a unix timestamp")?,
            "a unix timestamp",
        )?,
        None => created_at,
    };

    Ok(RateSnapshot {
        event_id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        published_at,
        source: optional(event, "source")?,
        rates: parse_content(&event.content)?,
    })
}

/// The `{"BTC": {code: price}}` object of the content, checked entry by
/// entry.
fn parse_content(content: &str) -> Result<BTreeMap<String, f64>, ParseError> {
    let invalid = |reason: String| ParseError::InvalidContent { reason };

    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|error| invalid(format!("not JSON: {error}")))?;
    let btc = value
        .get("BTC")
        .ok_or_else(|| invalid("no `BTC` object".to_string()))?;
    let table = btc
        .as_object()
        .ok_or_else(|| invalid("`BTC` is not an object of currency → price".to_string()))?;

    let mut rates = BTreeMap::new();
    for (code, price) in table {
        let price = price
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| invalid(format!("`{code}` = {price}: expected a positive number")))?;
        if code.trim().is_empty() {
            return Err(invalid("a currency code is blank".to_string()));
        }
        rates.insert(code.clone(), price);
    }

    if rates.is_empty() {
        return Err(invalid("`BTC` names no currency".to_string()));
    }

    Ok(rates)
}

#[cfg(test)]
mod tests;
