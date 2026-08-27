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

/// How far `published_at` may sit from the event's own `created_at`.
///
/// Both captured snapshots set the two clocks to the same second, and the
/// `expiration` tag gives a snapshot ten minutes of validity — so an event
/// signed more than that after its rates were fetched is publishing a figure
/// it considers stale itself. The window absorbs a slow publish or a little
/// clock drift; it does not license a backdate, which on an addressable kind
/// would let a snapshot signed today land in a period already reported.
pub(crate) const MAX_CLOCK_DIVERGENCE_SECS: i64 = 600;

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
        Some(value) => {
            let claimed = non_negative(
                "published_at",
                number::<i64>("published_at", &value, "a unix timestamp")?,
                "a unix timestamp",
            )?;
            near(created_at, claimed)?
        }
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

/// A currency code as the producers publish it: exactly three uppercase
/// ASCII letters, the ISO 4217 shape.
///
/// Rejected rather than folded: `usd` and `USD ` would be stored as
/// currencies of their own, never match the `USD` an order is denominated
/// in, and report a missing rate for a rate that was published. All 141
/// currencies of both captured snapshots have this shape, and it is the same
/// rule `[report].reference_currency` is held to.
fn canonical_code(code: &str) -> Result<String, ParseError> {
    let canonical = code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase());
    if canonical {
        Ok(code.to_string())
    } else {
        Err(ParseError::InvalidContent {
            reason: format!("`{code}`: expected a three-letter uppercase currency code"),
        })
    }
}

/// A `published_at` the signed clock corroborates.
///
/// The tag is the instance's own claim about when it fetched the rates,
/// while `created_at` is covered by the signature. Phase 3 orders snapshots
/// by `published_at`, so a claim the signature does not corroborate is not
/// evidence of when the price held.
fn near(created_at: i64, published_at: i64) -> Result<i64, ParseError> {
    if (published_at - created_at).abs() > MAX_CLOCK_DIVERGENCE_SECS {
        return Err(ParseError::OutOfRange {
            tag: "published_at",
            value: published_at.to_string(),
            expected: "a time within ten minutes of the event's own clock",
        });
    }
    Ok(published_at)
}

/// The `{"BTC": {code: price}}` object of the content, checked entry by
/// entry.
fn parse_content(content: &str) -> Result<BTreeMap<String, f64>, ParseError> {
    let invalid = |reason: String| ParseError::InvalidContent { reason };

    let parsed: Content = serde_json::from_str(content).map_err(|error| {
        // A syntax error is about the bytes; anything else is about what
        // they said, and that message is already the useful one.
        if error.is_syntax() || error.is_eof() {
            invalid(format!("not JSON: {error}"))
        } else {
            invalid(error.to_string())
        }
    })?;
    let table = parsed
        .btc
        .ok_or_else(|| invalid("no `BTC` object".to_string()))?;

    let mut rates = BTreeMap::new();
    for (code, price) in table {
        let price = price
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| invalid(format!("`{code}` = {price}: expected a positive number")))?;
        rates.insert(canonical_code(&code)?, price);
    }

    if rates.is_empty() {
        return Err(invalid("`BTC` names no currency".to_string()));
    }

    Ok(rates)
}

#[cfg(test)]
mod tests;

/// The content, read so that a member appearing twice is an error.
///
/// `serde_json::Value` keeps the last of two equal members, so
/// `{"BTC":{"USD":50000,"USD":1}}` would parse and store `1`: the figure
/// would depend on member order, and two readers of the same signed payload
/// could disagree about what it says. A parser that treats a non-positive
/// price as fatal cannot be relaxed about that.
struct Content {
    btc: Option<BTreeMap<String, serde_json::Value>>,
}

impl<'de> serde::Deserialize<'de> for Content {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ContentVisitor;

        impl<'de> serde::de::Visitor<'de> for ContentVisitor {
            type Value = Content;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object with a `BTC` table of currency → price")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Content, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut seen = std::collections::BTreeSet::new();
                let mut btc = None;

                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!(
                            "`{key}` appears more than once"
                        )));
                    }
                    if key == "BTC" {
                        btc = Some(map.next_value::<UniqueTable>()?.0);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }

                Ok(Content { btc })
            }
        }

        deserializer.deserialize_map(ContentVisitor)
    }
}

/// An object of currency → price in which no currency appears twice.
struct UniqueTable(BTreeMap<String, serde_json::Value>);

impl<'de> serde::Deserialize<'de> for UniqueTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TableVisitor;

        impl<'de> serde::de::Visitor<'de> for TableVisitor {
            type Value = UniqueTable;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("`BTC` to be an object of currency → price")
            }

            fn visit_map<A>(self, mut map: A) -> Result<UniqueTable, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut table = BTreeMap::new();
                while let Some((code, price)) = map.next_entry::<String, serde_json::Value>()? {
                    if table.insert(code.clone(), price).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "`{code}` appears more than once"
                        )));
                    }
                }
                Ok(UniqueTable(table))
            }
        }

        deserializer.deserialize_map(TableVisitor)
    }
}
