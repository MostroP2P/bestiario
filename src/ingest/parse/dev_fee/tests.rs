//! Parsing kind 8383, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// The tags a valid dev fee needs, as a starting point for breaking one.
fn valid_tags() -> Vec<(&'static str, &'static str)> {
    vec![
        ("order-id", "048b9483-15a3-4938-aec7-f60da7f14c8e"),
        ("amount", "116"),
        (
            "hash",
            "3d3695517badfa7809525b8ea96718bd697b59ba85209669b8063657d1ca9eb5",
        ),
        ("destination", "fund@walletofsatoshi.com"),
        ("network", "mainnet"),
        ("z", "dev-fee-payment"),
    ]
}

/// The valid tag set with one tag replaced, or — when `value` is `None` —
/// removed.
fn dev_fee_but(tag: &str, value: Option<&str>) -> Event {
    let mut tags: Vec<(String, String)> = valid_tags()
        .into_iter()
        .filter(|(name, _)| *name != tag)
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    if let Some(value) = value {
        tags.push((tag.to_string(), value.to_string()));
    }

    EventBuilder::new(Kind::from_u16(KIND), "")
        .tags(
            tags.iter()
                .map(|(name, value)| Tag::parse([name.as_str(), value.as_str()]).expect("tag")),
        )
        .finalize(&Keys::generate())
        .expect("signing")
}

#[test]
fn a_captured_dev_fee_parses_every_field() {
    let event = load(KIND, "typical");

    let fee = parse(&event).expect("a captured dev fee should parse");

    assert_eq!(fee.event_id, event.id.to_hex());
    assert_eq!(fee.pubkey, event.pubkey.to_hex());
    assert_eq!(fee.created_at, event.created_at.as_secs() as i64);
    assert!(fee.amount_sats > 0, "{}", fee.amount_sats);
    assert_eq!(fee.payment_hash.len(), 64, "{}", fee.payment_hash);
    assert!(fee.destination.is_some());
    assert_eq!(fee.network, Some(Network::Mainnet));
}

#[test]
fn two_instances_produce_two_distinct_fees() {
    // Cheap guard against a parser that reads something from the wrong place:
    // the two captured fees come from different instances, for different
    // orders, and must not collapse into each other.
    let one = parse(&load(KIND, "typical")).expect("typical");
    let other = parse(&load(KIND, "another_instance")).expect("another instance");

    assert_ne!(one.pubkey, other.pubkey);
    assert_ne!(one.order_id, other.order_id);
    assert_ne!(one.payment_hash, other.payment_hash);
}

#[test]
fn a_fee_without_a_destination_still_parses() {
    // The destination says where the payment went, not that it happened.
    // Refusing the event over it would drop a settlement from the volume
    // figures to avoid missing a label.
    let fee = parse(&dev_fee_but("destination", None)).expect("destination is optional");

    assert_eq!(fee.destination, None);
}

#[test]
fn a_fee_without_a_network_still_parses() {
    let fee = parse(&dev_fee_but("network", None)).expect("network is optional");

    assert_eq!(fee.network, None);
}

#[test]
fn an_unknown_network_is_an_error() {
    let error = parse(&dev_fee_but("network", Some("mutinynet"))).expect_err("unknown network");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "network", .. }),
        "{error}"
    );
}

#[test]
fn each_required_tag_is_named_when_it_is_missing() {
    for tag in ["order-id", "amount", "hash"] {
        let error = parse(&dev_fee_but(tag, None)).expect_err(tag);

        assert_eq!(error, ParseError::MissingTag { tag }, "removing {tag}");
    }
}

#[test]
fn a_non_numeric_amount_is_an_error() {
    let error = parse(&dev_fee_but("amount", Some("some"))).expect_err("non-numeric amount");

    assert!(
        matches!(error, ParseError::NotANumber { tag: "amount", .. }),
        "{error}"
    );
}

#[test]
fn an_event_of_another_kind_is_rejected_before_any_tag_is_read() {
    let error = parse(&load(38383, "success")).expect_err("an order is not a dev fee");

    assert_eq!(
        error,
        ParseError::WrongKind {
            expected: KIND,
            found: 38383,
        }
    );
}
