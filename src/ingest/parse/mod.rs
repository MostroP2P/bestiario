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
pub mod info;
pub mod order;
pub mod rates;
pub mod relay_list;

#[cfg(test)]
pub(crate) mod fixtures;

use nostr_sdk::prelude::Event;

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
