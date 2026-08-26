//! Parser for kind 38383 — orders (`docs/SPEC.md` §2.1).
//!
//! Orders are where every number in bestiario starts: counts, volumes,
//! currencies, payment methods and premiums all come from this kind. The
//! parser therefore refuses to guess. In particular:
//!
//! - an unknown `s` is an error, not a fifth status folded into `canceled`;
//! - `fa` is either one value or a `[min, max]` pair, and anything else is an
//!   error rather than a silently truncated range;
//! - `expires_at` is required. Across the capture behind `tests/fixtures` it
//!   was published by every one of the 172 Mostro orders and by none of the
//!   28 orders from other platforms, and only Mostro orders reach a parser
//!   (`docs/SPEC.md` §8.1 step 4 rejects the rest).

use nostr_sdk::prelude::Event;

use super::{
    ParseError, expect_discriminator, expect_kind, finite, non_blank, non_negative, number,
    optional_network, required, tag_values, uuid,
};
use crate::network::Network;

/// The kind this parser accepts.
pub const KIND: u16 = 38383;

/// The value of the `z` tag an order carries.
pub const DISCRIMINATOR: &str = "order";

/// Which side the *maker* is on, as published in the `k` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Buy,
    Sell,
}

/// The four statuses that reach the wire (`docs/SPEC.md` §7).
///
/// mostrod has more internal statuses than these; `expired`,
/// `canceled-by-admin` and `cooperatively-canceled` all arrive collapsed into
/// `canceled`, and a disputed order stays `in-progress`. The type says only
/// what is observable, so that nothing downstream can claim otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    InProgress,
    Success,
    Canceled,
}

/// The fiat side of an order: one amount, or the bounds of a range order.
///
/// A range order publishes `fa = [min, max]` while it is `pending` and
/// collapses to a single value once taken, so both shapes are legal for the
/// same order at different times and the type has to carry which one it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FiatAmount {
    Fixed(f64),
    Range { min: f64, max: f64 },
}

/// One published version of an order — one 38383 event, parsed.
///
/// Mirrors `order_versions` in `docs/SPEC.md` §4. The payment methods stay a
/// list here and are joined into the csv the column holds at persistence
/// time, so the parser never has to invent an escaping rule for a method that
/// contains a comma.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderVersion {
    pub event_id: String,
    pub order_id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub direction: Direction,
    pub status: Status,
    pub fiat_code: String,
    pub amount_sats: i64,
    pub fiat: FiatAmount,
    pub payment_methods: Vec<String>,
    pub premium: f64,
    pub network: Option<Network>,
    pub expires_at: i64,
}

impl Direction {
    /// The wire form, as it appears in the `k` tag and in the `kind` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "buy" => Ok(Self::Buy),
            "sell" => Ok(Self::Sell),
            _ => Err(ParseError::UnknownValue {
                tag: "k",
                value: value.to_string(),
                expected: "`buy` or `sell`",
            }),
        }
    }
}

impl Status {
    /// The wire form, as it appears in the `s` tag and in the `status` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in-progress",
            Self::Success => "success",
            Self::Canceled => "canceled",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "pending" => Ok(Self::Pending),
            "in-progress" => Ok(Self::InProgress),
            "success" => Ok(Self::Success),
            "canceled" => Ok(Self::Canceled),
            _ => Err(ParseError::UnknownValue {
                tag: "s",
                value: value.to_string(),
                expected: "`pending`, `in-progress`, `success` or `canceled`",
            }),
        }
    }
}

impl FiatAmount {
    /// The amount a fixed order trades, or `None` for a range that has not
    /// collapsed yet — the shape the `fiat_amount` column stores.
    pub fn fixed(self) -> Option<f64> {
        match self {
            Self::Fixed(amount) => Some(amount),
            Self::Range { .. } => None,
        }
    }

    /// The bounds of a range order, or `None` for a fixed amount.
    pub fn bounds(self) -> Option<(f64, f64)> {
        match self {
            Self::Fixed(_) => None,
            Self::Range { min, max } => Some((min, max)),
        }
    }
}

/// Turn a 38383 event into an [`OrderVersion`].
pub fn parse(event: &Event) -> Result<OrderVersion, ParseError> {
    expect_kind(event, KIND)?;
    expect_discriminator(event, DISCRIMINATOR)?;

    Ok(OrderVersion {
        event_id: event.id.to_hex(),
        order_id: uuid("d", &required(event, "d")?)?,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs() as i64,
        direction: Direction::parse(&required(event, "k")?)?,
        status: Status::parse(&required(event, "s")?)?,
        fiat_code: required(event, "f")?,
        amount_sats: non_negative(
            "amt",
            number("amt", &required(event, "amt")?, "an amount in sats")?,
            "an amount in sats",
        )?,
        fiat: parse_fiat_amount(event)?,
        payment_methods: parse_payment_methods(event)?,
        premium: finite("premium", &required(event, "premium")?, "a percentage")?,
        network: optional_network(event)?,
        expires_at: non_negative(
            "expires_at",
            number::<i64>(
                "expires_at",
                &required(event, "expires_at")?,
                "a unix timestamp",
            )?,
            "a unix timestamp",
        )?,
    })
}

/// `fa` with one value is an amount; with two it is the `[min, max]` of a
/// pending range order. Any other count is malformed.
fn parse_fiat_amount(event: &Event) -> Result<FiatAmount, ParseError> {
    let values = tag_values(event, "fa")?.ok_or(ParseError::MissingTag { tag: "fa" })?;

    match values.as_slice() {
        [] => Err(ParseError::EmptyTag { tag: "fa" }),
        [amount] => Ok(FiatAmount::Fixed(fiat_amount(amount, "a fiat amount")?)),
        [min, max] => {
            let min = fiat_amount(min, "the minimum of a range")?;
            let max = fiat_amount(max, "the maximum of a range")?;

            // A range order is an invitation to pick a number between the
            // two. Inverted, it invites nothing, and every later comparison
            // against it — does this order cover 10 000 ARS? — silently
            // answers no.
            if min > max {
                return Err(ParseError::InvertedRange {
                    tag: "fa",
                    min,
                    max,
                });
            }
            Ok(FiatAmount::Range { min, max })
        }
        values => Err(ParseError::WrongValueCount {
            tag: "fa",
            count: values.len(),
            expected: "one amount, or a `[min, max]` pair",
        }),
    }
}

/// `pm` carries one value per method, all in a single tag.
fn parse_payment_methods(event: &Event) -> Result<Vec<String>, ParseError> {
    let values = tag_values(event, "pm")?.ok_or(ParseError::MissingTag { tag: "pm" })?;

    if values.is_empty() {
        return Err(ParseError::EmptyTag { tag: "pm" });
    }
    values
        .into_iter()
        .map(|method| non_blank("pm", method, "a payment method"))
        .collect()
}

/// A fiat amount is a quantity of money: finite, and never negative.
fn fiat_amount(value: &str, expected: &'static str) -> Result<f64, ParseError> {
    non_negative("fa", finite("fa", value, expected)?, expected)
}

#[cfg(test)]
mod tests;
