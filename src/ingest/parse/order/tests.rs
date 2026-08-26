//! Parsing kind 38383, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// Build a 38383 with the given tags, signed by a throwaway key.
///
/// Used only for the malformed cases: everything that is supposed to work is
/// tested against a real captured event instead, so that a passing test means
/// the parser survives the network rather than the test author's idea of it.
fn order_with(tags: &[(&str, Vec<&str>)]) -> Event {
    let tags = tags.iter().map(|(name, values)| {
        Tag::parse(
            std::iter::once(*name)
                .chain(values.iter().copied())
                .collect::<Vec<_>>(),
        )
        .expect("well-formed tag")
    });

    EventBuilder::new(Kind::from_u16(KIND), "")
        .tags(tags)
        .finalize(&Keys::generate())
        .expect("signing")
}

/// The tags a valid order needs, as a starting point for breaking one of them.
fn valid_tags() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("d", vec!["3c2548e4-d4f6-43dd-9d49-cffe71db806b"]),
        ("k", vec!["sell"]),
        ("f", vec!["ARS"]),
        ("s", vec!["pending"]),
        ("amt", vec!["0"]),
        ("fa", vec!["25000"]),
        ("pm", vec!["Mercado Pago"]),
        ("premium", vec!["2"]),
        ("network", vec!["mainnet"]),
        ("expires_at", vec!["1787824986"]),
        ("y", vec!["mostro", "Mostro"]),
        ("z", vec!["order"]),
    ]
}

/// The valid tag set with one tag replaced, or — when `values` is empty —
/// removed.
fn order_but(tag: &str, values: &[&'static str]) -> Event {
    let mut tags = valid_tags();
    tags.retain(|(name, _)| *name != tag);
    if !values.is_empty() {
        let tag: &'static str = valid_tags()
            .into_iter()
            .map(|(name, _)| name)
            .find(|name| *name == tag)
            .unwrap_or("unknown");
        tags.push((tag, values.to_vec()));
    }
    order_with(&tags)
}

#[test]
fn a_fixed_amount_order_parses_every_field() {
    let event = load(KIND, "pending_market_price");

    let order = parse(&event).expect("a captured order should parse");

    assert_eq!(order.event_id, event.id.to_hex());
    assert_eq!(order.pubkey, event.pubkey.to_hex());
    assert_eq!(order.created_at, event.created_at.as_secs() as i64);
    assert_eq!(order.status, Status::Pending);
    assert_eq!(order.network, Some(Network::Mainnet));
    assert!(order.fiat.fixed().is_some(), "{:?}", order.fiat);
    assert!(order.expires_at > 0);
}

#[test]
fn a_range_order_keeps_both_bounds() {
    // The reason FiatAmount is an enum: a range order publishes two values
    // while pending, and collapsing them to one here would lose the range for
    // good — order_versions is the only record of what was published.
    let order = parse(&load(KIND, "pending_range")).expect("range order");

    let (min, max) = order.fiat.bounds().expect("a range order has bounds");
    assert!(min < max, "{min} .. {max}");
    assert_eq!(order.fiat.fixed(), None);
}

#[test]
fn a_range_order_may_still_carry_a_non_zero_amount() {
    // `fa = [min, max]` with `amt` set: the maker fixed the sats and left the
    // fiat to the taker. Reading `amt = 0` out of a range order would drop
    // this one from the volume figures.
    let order = parse(&load(KIND, "pending_range_with_fixed_sats")).expect("priced range order");

    assert!(order.fiat.bounds().is_some(), "{:?}", order.fiat);
    assert!(order.amount_sats > 0, "{}", order.amount_sats);
}

#[test]
fn a_market_price_order_parses_with_a_zero_amount() {
    // `amt = 0` means "price at market when taken", not "worth nothing": it
    // has to parse, and the volume statistics have to know to skip it.
    let order = parse(&load(KIND, "pending_market_price")).expect("market-price order");

    assert_eq!(order.amount_sats, 0);
}

#[test]
fn every_payment_method_of_a_multi_method_order_survives() {
    let order = parse(&load(KIND, "pending_multiple_payment_methods")).expect("multi-method order");

    assert!(
        order.payment_methods.len() > 1,
        "{:?}",
        order.payment_methods
    );
    assert!(
        order.payment_methods.iter().all(|m| !m.is_empty()),
        "{:?}",
        order.payment_methods
    );
}

#[test]
fn every_captured_status_parses_to_its_own_variant() {
    let cases = [
        ("pending_market_price", Status::Pending),
        ("in_progress", Status::InProgress),
        ("success", Status::Success),
        ("canceled", Status::Canceled),
    ];

    for (fixture, expected) in cases {
        let order = parse(&load(KIND, fixture)).expect(fixture);
        assert_eq!(order.status, expected, "{fixture}");
    }
}

#[test]
fn a_non_mainnet_order_still_parses() {
    // Filtering by network is the pipeline's job (SPEC 8.1 step 5), not the
    // parser's: a regtest order is well formed, it is simply not counted.
    let order = parse(&load(KIND, "success")).expect("regtest order");

    assert_eq!(order.network, Some(Network::Regtest));
}

#[test]
fn the_maker_rating_tag_is_ignored_without_failing() {
    // Reputation is out of scope (SPEC 1), but an order that carries a rating
    // is still an order and still counts.
    let order = parse(&load(KIND, "with_maker_rating")).expect("rated order");

    assert_eq!(order.fiat_code, "BRL");
}

#[test]
fn an_order_from_another_platform_is_rejected_for_its_missing_expires_at() {
    // Not the platform filter — that is the pipeline's (SPEC 8.1 step 4).
    // This pins the reason the parser can require `expires_at` at all: the
    // orders that omit it are exactly the ones that never reach it.
    let error = parse(&load(KIND, "other_platform_bitway")).expect_err("no expires_at");

    assert_eq!(error, ParseError::MissingTag { tag: "expires_at" });
}

#[test]
fn an_order_whose_d_is_not_a_uuid_is_rejected() {
    // hodlhodl and the telegram bots key their NIP-69 orders on their own
    // internal ids. `d` is the natural key of an order here and of the
    // projection built from it, so accepting a non-UUID would let unrelated
    // events — two empty ids above all — merge into one order.
    for fixture in ["other_platform_hodlhodl", "other_platform_telegram"] {
        let error = parse(&load(KIND, fixture)).expect_err(fixture);

        assert!(
            matches!(error, ParseError::UnknownValue { tag: "d", .. }),
            "{fixture}: {error}"
        );
    }

    let error = parse(&order_but("d", &[""])).expect_err("empty d");
    assert!(
        matches!(error, ParseError::UnknownValue { tag: "d", .. }),
        "{error}"
    );
}

#[test]
fn a_non_finite_number_is_rejected_rather_than_poisoning_every_sum() {
    // `f64::from_str` accepts all three of these. One NaN premium would make
    // every average computed with it NaN, and SQLite stores a non-finite
    // float as NULL, so the value would not even survive to be noticed.
    for value in ["NaN", "inf", "-inf"] {
        let error = parse(&order_but("premium", &[value])).expect_err(value);
        assert!(
            matches!(error, ParseError::NotANumber { tag: "premium", .. }),
            "{value}: {error}"
        );

        let error = parse(&order_but("fa", &[value])).expect_err(value);
        assert!(
            matches!(error, ParseError::NotANumber { tag: "fa", .. }),
            "{value}: {error}"
        );
    }
}

#[test]
fn an_event_without_the_order_discriminator_is_rejected() {
    // The kind says which parser to use; `z` says what the publisher meant
    // the event to be. Every captured order agrees on both.
    let error = parse(&order_but("z", &[])).expect_err("no z");
    assert_eq!(error, ParseError::MissingTag { tag: "z" });

    let error = parse(&order_but("z", &["dispute"])).expect_err("wrong z");
    assert!(
        matches!(error, ParseError::UnknownValue { tag: "z", .. }),
        "{error}"
    );
}

#[test]
fn an_event_of_another_kind_is_rejected_before_any_tag_is_read() {
    let error = parse(&load(38386, "status_settled")).expect_err("a dispute is not an order");

    assert_eq!(
        error,
        ParseError::WrongKind {
            expected: KIND,
            found: 38386,
        }
    );
}

#[test]
fn an_unknown_status_is_an_error_and_not_a_fifth_bucket() {
    let error = parse(&order_but("s", &["disputed"])).expect_err("unknown status");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "s", ref value, .. } if value == "disputed"),
        "{error}"
    );
}

#[test]
fn an_unknown_direction_is_an_error() {
    let error = parse(&order_but("k", &["swap"])).expect_err("unknown direction");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "k", .. }),
        "{error}"
    );
}

#[test]
fn an_unknown_network_is_an_error_rather_than_a_silent_none() {
    // A network nobody has heard of must not end up counted as mainnet, and
    // must not vanish into a `None` that reads like "not published".
    let error = parse(&order_but("network", &["mutinynet"])).expect_err("unknown network");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "network", .. }),
        "{error}"
    );
}

#[test]
fn a_three_valued_fa_is_an_error_rather_than_a_truncated_range() {
    let error = parse(&order_but("fa", &["1", "2", "3"])).expect_err("three-valued fa");

    assert_eq!(
        error,
        ParseError::WrongValueCount {
            tag: "fa",
            count: 3,
            expected: "one amount, or a `[min, max]` pair",
        }
    );
}

#[test]
fn a_non_numeric_amount_is_an_error() {
    let error = parse(&order_but("amt", &["lots"])).expect_err("non-numeric amt");

    assert!(
        matches!(error, ParseError::NotANumber { tag: "amt", .. }),
        "{error}"
    );
}

#[test]
fn each_required_tag_is_named_when_it_is_missing() {
    for tag in [
        "d",
        "k",
        "f",
        "s",
        "amt",
        "fa",
        "pm",
        "premium",
        "expires_at",
    ] {
        let error = parse(&order_but(tag, &[])).expect_err(tag);

        assert_eq!(error, ParseError::MissingTag { tag }, "removing {tag}");
    }
}

#[test]
fn a_missing_network_parses_to_none_because_the_column_allows_it() {
    let order = parse(&order_but("network", &[])).expect("network is optional");

    assert_eq!(order.network, None);
}
