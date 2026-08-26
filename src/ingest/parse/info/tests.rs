//! Parsing kind 38385, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// An info event published by `keys`, with the given tags.
fn info_with(keys: &Keys, tags: &[(&str, &str)]) -> Event {
    EventBuilder::new(Kind::from_u16(KIND), "")
        .tags(
            tags.iter()
                .map(|(name, value)| Tag::parse([*name, *value]).expect("well-formed tag")),
        )
        .finalize(keys)
        .expect("signing")
}

/// A valid info event, optionally with one tag replaced.
fn info_but(tag: &str, value: Option<&str>) -> Event {
    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let valid: Vec<(String, String)> = vec![
        ("d".to_string(), pubkey),
        ("fee".to_string(), "0.006".to_string()),
        ("max_order_amount".to_string(), "3000000".to_string()),
        ("min_order_amount".to_string(), "100".to_string()),
        ("bond_enabled".to_string(), "true".to_string()),
        ("z".to_string(), "info".to_string()),
    ];
    let mut tags: Vec<(String, String)> =
        valid.into_iter().filter(|(name, _)| name != tag).collect();
    if let Some(value) = value {
        tags.push((tag.to_string(), value.to_string()));
    }

    let borrowed: Vec<(&str, &str)> = tags
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    info_with(&keys, &borrowed)
}

#[test]
fn a_captured_info_event_parses_every_field() {
    let event = load(KIND, "typical");

    let info = parse(&event).expect("a captured info event should parse");

    assert_eq!(info.event_id, event.id.to_hex());
    assert_eq!(info.pubkey, event.pubkey.to_hex());
    assert_eq!(info.fee, Some(0.006));
    assert_eq!(info.protocol_version.as_deref(), Some("2"));
    assert_eq!(info.bond_enabled, Some(true));
    assert!(info.max_order_amount > info.min_order_amount);
    assert!(
        info.fiat_currencies
            .as_deref()
            .is_some_and(|c| c.contains(',')),
        "{:?}",
        info.fiat_currencies
    );
}

#[test]
fn a_zero_fee_instance_parses_with_a_fee_of_zero() {
    // An instance with `fee = 0` never emits a dev fee (SPEC 2.2), so this is
    // the difference between "charges nothing" and "we do not know what it
    // charges" — and phase 3 must not read the first as the second.
    let info = parse(&load(KIND, "zero_fee")).expect("zero-fee instance");

    assert_eq!(info.fee, Some(0.0));
    assert_eq!(info.bond_enabled, Some(false));
}

#[test]
fn an_instance_that_publishes_no_protocol_version_still_parses() {
    // Two of the twenty instances captured omit it. An info event is a
    // self-description that grows tag by tag across releases; refusing it over
    // a field it never promised would cost the whole instance.
    let info = parse(&load(KIND, "without_protocol_version")).expect("no protocol_version");

    assert_eq!(info.protocol_version, None);
    assert!(info.fee.is_some(), "the rest of the event still parses");
}

#[test]
fn an_instance_without_a_bond_policy_still_parses() {
    let info = parse(&load(KIND, "without_protocol_version")).expect("no bond policy");

    assert_eq!(info.bond_enabled, None);
}

#[test]
fn a_nameless_instance_parses_like_any_other() {
    let info = parse(&load(KIND, "without_instance_name")).expect("nameless instance");

    assert!(info.mostro_version.is_some());
}

#[test]
fn the_d_tag_must_be_the_publishers_own_pubkey() {
    // A 38385 whose identifier is somebody else's pubkey does not describe
    // its publisher, and nothing downstream could say whose fee it holds.
    let error = parse(&info_but(
        "d",
        Some("0000000000000000000000000000000000000000000000000000000000000000"),
    ))
    .expect_err("d is another pubkey");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "d", .. }),
        "{error}"
    );
}

#[test]
fn a_missing_d_tag_is_named() {
    let error = parse(&info_but("d", None)).expect_err("no d");

    assert_eq!(error, ParseError::MissingTag { tag: "d" });
}

#[test]
fn an_unreadable_fee_is_an_error_rather_than_an_absent_one() {
    // Phase 3 divides by this number to infer volume from a dev fee.
    let error = parse(&info_but("fee", Some("nothing"))).expect_err("unreadable fee");

    assert!(
        matches!(error, ParseError::NotANumber { tag: "fee", .. }),
        "{error}"
    );
}

#[test]
fn an_unrecognised_boolean_is_an_error_rather_than_a_silent_false() {
    let error = parse(&info_but("bond_enabled", Some("yes"))).expect_err("not a boolean");

    assert!(
        matches!(
            error,
            ParseError::UnknownValue {
                tag: "bond_enabled",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn an_event_of_another_kind_is_rejected_before_any_tag_is_read() {
    let error = parse(&load(38386, "status_settled")).expect_err("a dispute is not an info event");

    assert_eq!(
        error,
        ParseError::WrongKind {
            expected: KIND,
            found: 38386,
        }
    );
}
