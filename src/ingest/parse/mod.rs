//! One parser per Nostr kind: tags in, typed struct out.
//!
//! Responsibility: the tag-level knowledge of `docs/SPEC.md` §2. A parser
//! never touches the database and never decides whether an event is wanted —
//! it only decides whether an event is *well formed*. Unknown or missing
//! required tags are hard errors, never silent defaults.
//!
//! # Why hard errors
//!
//! Every value parsed here ends up in a count, a sum or an average. A parser
//! that tolerates the unexpected — an unknown status folded into `canceled`,
//! a missing amount read as zero — does not fail, it answers wrongly and
//! keeps answering wrongly for as long as nobody re-derives the numbers by
//! hand. A rejected event is logged and stays in `events.raw_json`, so the
//! cost of being strict is one reprocessing run; the cost of being lenient is
//! a statistic nobody can trust.

pub mod dev_fee;
pub mod dispute;
pub mod identity;
pub mod info;
pub mod order;
pub mod rates;
pub mod relay_list;

pub use identity::{MOSTRO, instance_name, is_mostro, platform};

#[cfg(test)]
pub(crate) mod fixtures;

use nostr_sdk::prelude::Event;

use crate::network::Network;

/// Why an event could not be turned into a typed value.
///
/// Each variant names the offending tag and what was expected of it: a
/// rejection is only useful if the log line says which event, which tag, and
/// what the parser wanted instead.
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error("expected kind {expected}, got {found}")]
    WrongKind { expected: u16, found: u16 },

    #[error("missing `{tag}` tag")]
    MissingTag { tag: &'static str },

    #[error("`{tag}` tag carries no value")]
    EmptyTag { tag: &'static str },

    #[error("`{tag}` = `{value}`: expected {expected}")]
    UnknownValue {
        tag: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("`{tag}` = `{value}`: expected {expected}")]
    NotANumber {
        tag: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("`{tag}` carries {count} values: expected {expected}")]
    WrongValueCount {
        tag: &'static str,
        count: usize,
        expected: &'static str,
    },

    #[error("`{tag}` appears {count} times: expected it once")]
    RepeatedTag { tag: &'static str, count: usize },

    #[error("`{tag}` is blank: expected {expected}")]
    BlankValue {
        tag: &'static str,
        expected: &'static str,
    },

    #[error("`{tag}` = `{value}`: expected {expected}")]
    OutOfRange {
        tag: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("`{tag}` = `[{min}, {max}]`: expected the minimum not to exceed the maximum")]
    InvertedRange {
        tag: &'static str,
        min: f64,
        max: f64,
    },
}

/// The values of every occurrence of the `name` tag.
///
/// Nostr allows a tag to repeat. Only 10002 uses that (one `r` per relay);
/// for every other kind a repeat means two answers to a question that has
/// one, which is why [`tag_values`] refuses to pick between them.
pub(crate) fn repeated_tag_values(event: &Event, name: &str) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .filter(|values| values.first().map(String::as_str) == Some(name))
        .map(|values| values[1..].to_vec())
        .collect()
}

/// The values of the `name` tag, or `None` if the event has no such tag.
///
/// A repeated tag is an error rather than a first-one-wins: an event that
/// publishes `d` twice has no single natural key, and silently taking the
/// first would let the second say anything at all.
pub(crate) fn tag_values(
    event: &Event,
    name: &'static str,
) -> Result<Option<Vec<String>>, ParseError> {
    let mut occurrences = repeated_tag_values(event, name);

    match occurrences.len() {
        0 => Ok(None),
        1 => Ok(Some(occurrences.remove(0))),
        count => Err(ParseError::RepeatedTag { tag: name, count }),
    }
}

/// The single, non-blank value of a required tag.
///
/// A blank value is rejected rather than stored: `["f", ""]` would open a
/// currency bucket named after nothing, and `["pm", ""]` a payment method
/// nobody offers.
pub(crate) fn required(event: &Event, tag: &'static str) -> Result<String, ParseError> {
    let values = tag_values(event, tag)?.ok_or(ParseError::MissingTag { tag })?;
    let value = match values.len() {
        0 => return Err(ParseError::EmptyTag { tag }),
        1 => values.into_iter().next().expect("length checked"),
        count => {
            return Err(ParseError::WrongValueCount {
                tag,
                count,
                expected: "exactly one value",
            });
        }
    };

    non_blank(tag, value, "a value")
}

/// The single value of a tag that may be absent.
///
/// A tag published with a blank value reads as absent. Real instances do
/// publish `["fiat_currencies_accepted", ""]` and `["lnd_uris", ""]`, and
/// "published nothing" and "published an empty string" are not two different
/// answers worth storing apart.
pub(crate) fn optional(event: &Event, tag: &'static str) -> Result<Option<String>, ParseError> {
    let Some(values) = tag_values(event, tag)? else {
        return Ok(None);
    };
    let value = match values.len() {
        0 => return Ok(None),
        1 => values.into_iter().next().expect("length checked"),
        count => {
            return Err(ParseError::WrongValueCount {
                tag,
                count,
                expected: "exactly one value",
            });
        }
    };

    Ok(Some(value).filter(|value| !value.trim().is_empty()))
}

/// Reject a value that is empty or nothing but whitespace.
pub(crate) fn non_blank(
    tag: &'static str,
    value: String,
    expected: &'static str,
) -> Result<String, ParseError> {
    if value.trim().is_empty() {
        Err(ParseError::BlankValue { tag, expected })
    } else {
        Ok(value)
    }
}

/// Reject a quantity that cannot be negative — an amount of money, or a
/// timestamp. A negative one does not just look wrong, it *subtracts* from
/// the volume figures it lands in.
pub(crate) fn non_negative<T>(
    tag: &'static str,
    value: T,
    expected: &'static str,
) -> Result<T, ParseError>
where
    T: PartialOrd + Default + std::fmt::Display + Copy,
{
    if value < T::default() {
        Err(ParseError::OutOfRange {
            tag,
            value: value.to_string(),
            expected,
        })
    } else {
        Ok(value)
    }
}

/// Reject an event of the wrong kind before reading a single tag.
pub(crate) fn expect_kind(event: &Event, expected: u16) -> Result<(), ParseError> {
    let found = event.kind.as_u16();
    if found == expected {
        Ok(())
    } else {
        Err(ParseError::WrongKind { expected, found })
    }
}

/// Parse a tag that has to be a *finite* number.
///
/// `f64::from_str` accepts `NaN`, `inf` and `-inf`, and any of the three would
/// pass silently through the parser and into a sum, an average or a
/// percentile, where it poisons every figure computed with it — and reaches
/// SQLite, which stores non-finite floats as NULL. A value that cannot be
/// added up is not a number this project has any use for.
pub(crate) fn finite(
    tag: &'static str,
    value: &str,
    expected: &'static str,
) -> Result<f64, ParseError> {
    let parsed: f64 = number(tag, value, expected)?;

    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(ParseError::NotANumber {
            tag,
            value: value.to_string(),
            expected,
        })
    }
}

/// Parse a tag that has to be a UUID, returning it in its canonical form.
///
/// `d` is the natural key of an order and of a dispute: it is what versions
/// are grouped by and what the projections are keyed on. A value that is not
/// a UUID — an empty string above all — would merge unrelated events into one
/// order, so the shape is checked here rather than trusted.
pub(crate) fn uuid(tag: &'static str, value: &str) -> Result<String, ParseError> {
    ::uuid::Uuid::parse_str(value)
        .map(|uuid| uuid.to_string())
        .map_err(|_| ParseError::UnknownValue {
            tag,
            value: value.to_string(),
            expected: "a UUID",
        })
}

/// Check the `z` discriminator of a kind that publishes one.
///
/// The kind number says which parser to use; `z` says what the publisher
/// meant the event to be. They agree on every event ever captured, and an
/// event where they disagree is one nobody has a use for.
pub(crate) fn expect_discriminator(
    event: &Event,
    expected: &'static str,
) -> Result<(), ParseError> {
    let found = required(event, "z")?;

    if found == expected {
        Ok(())
    } else {
        Err(ParseError::UnknownValue {
            tag: "z",
            value: found,
            expected,
        })
    }
}

/// Parse a required numeric tag, naming what the value should have looked like.
pub(crate) fn number<T: std::str::FromStr>(
    tag: &'static str,
    value: &str,
    expected: &'static str,
) -> Result<T, ParseError> {
    value.parse().map_err(|_| ParseError::NotANumber {
        tag,
        value: value.to_string(),
        expected,
    })
}

/// The `network` tag, shared by orders (38383) and dev fees (8383).
///
/// An unrecognised value is an error rather than a `None`: the network filter
/// of `docs/SPEC.md` §8.1 decides what to count, and a network it has never
/// heard of has to be seen by whoever runs the indexer instead of quietly
/// joining the mainnet figures.
pub(crate) fn optional_network(event: &Event) -> Result<Option<Network>, ParseError> {
    let Some(value) = optional(event, "network")? else {
        return Ok(None);
    };

    Network::from_wire(&value)
        .map(Some)
        .ok_or(ParseError::UnknownValue {
            tag: "network",
            value,
            expected: "`mainnet`, `testnet`, `signet` or `regtest`",
        })
}
