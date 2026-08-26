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
#[derive(Debug, thiserror::Error, PartialEq)]
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
}

/// Every value of the first `name` tag, or `None` if the event has no such tag.
///
/// Nostr allows repeated tags; Mostro does not use them, so the first
/// occurrence is the one that counts and a second would be invisible here.
pub(crate) fn tag_values(event: &Event, name: &str) -> Option<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .find(|values| values.first().map(String::as_str) == Some(name))
        .map(|values| values[1..].to_vec())
}

/// The single value of a required tag.
pub(crate) fn required(event: &Event, tag: &'static str) -> Result<String, ParseError> {
    let values = tag_values(event, tag).ok_or(ParseError::MissingTag { tag })?;
    match values.len() {
        0 => Err(ParseError::EmptyTag { tag }),
        1 => Ok(values.into_iter().next().expect("length checked")),
        count => Err(ParseError::WrongValueCount {
            tag,
            count,
            expected: "exactly one value",
        }),
    }
}

/// The single value of a tag that may be absent, but not empty if present.
pub(crate) fn optional(event: &Event, tag: &'static str) -> Result<Option<String>, ParseError> {
    match tag_values(event, tag) {
        None => Ok(None),
        Some(_) => required(event, tag).map(Some),
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

    match value.as_str() {
        "mainnet" => Ok(Some(Network::Mainnet)),
        "testnet" => Ok(Some(Network::Testnet)),
        "signet" => Ok(Some(Network::Signet)),
        "regtest" => Ok(Some(Network::Regtest)),
        _ => Err(ParseError::UnknownValue {
            tag: "network",
            value,
            expected: "`mainnet`, `testnet`, `signet` or `regtest`",
        }),
    }
}
