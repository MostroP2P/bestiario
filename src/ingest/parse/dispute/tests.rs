//! Parsing kind 38386, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// The valid tag set with one tag replaced, or — when `value` is `None` —
/// removed.
fn dispute_but(tag: &str, value: Option<&str>) -> Event {
    let valid = [
        ("d", "c6ebce7e-e521-4df3-a8c5-24301145eb66"),
        ("s", "initiated"),
        ("initiator", "seller"),
        ("created_at", "1787533512"),
        ("z", "dispute"),
    ];
    let mut tags: Vec<(String, String)> = valid
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
fn a_captured_dispute_parses_every_field() {
    let event = load(KIND, "status_initiated");

    let dispute = parse(&event).expect("a captured dispute should parse");

    assert_eq!(dispute.event_id, event.id.to_hex());
    assert_eq!(dispute.pubkey, event.pubkey.to_hex());
    assert_eq!(dispute.status, Status::Initiated);
    assert_eq!(dispute.initiator, Some(Initiator::Seller));
    assert!(dispute.opened_at.is_some());
}

#[test]
fn the_opened_at_tag_is_not_the_events_own_created_at() {
    // The two are different questions — when the dispute was opened, and when
    // this version of it was published — and every captured dispute answers
    // them differently. Reading one for the other would date every dispute to
    // its last state change.
    for fixture in [
        "status_in_progress",
        "status_seller_refunded",
        "status_settled",
    ] {
        let dispute = parse(&load(KIND, fixture)).expect(fixture);

        let opened_at = dispute.opened_at.expect("captured disputes carry the tag");
        assert!(
            opened_at < dispute.created_at,
            "{fixture}: opened at {opened_at}, published at {}",
            dispute.created_at
        );
    }
}

#[test]
fn every_captured_status_parses_to_its_own_variant() {
    let cases = [
        ("status_initiated", Status::Initiated),
        ("status_in_progress", Status::InProgress),
        ("status_seller_refunded", Status::SellerRefunded),
        ("status_settled", Status::Settled),
    ];

    for (fixture, expected) in cases {
        let dispute = parse(&load(KIND, fixture)).expect(fixture);
        assert_eq!(dispute.status, expected, "{fixture}");
    }
}

#[test]
fn a_dispute_from_a_nameless_instance_parses_like_any_other() {
    // The name lives in `y`, which this parser does not read; PR 11 does.
    // What matters here is that its absence is not an error.
    let dispute = parse(&load(KIND, "without_instance_name")).expect("nameless instance");

    assert_eq!(dispute.status, Status::SellerRefunded);
}

#[test]
fn an_unknown_status_is_an_error_and_not_a_sixth_bucket() {
    let error = parse(&dispute_but("s", Some("mediated"))).expect_err("unknown status");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "s", .. }),
        "{error}"
    );
}

#[test]
fn an_unknown_initiator_is_an_error() {
    let error = parse(&dispute_but("initiator", Some("admin"))).expect_err("unknown initiator");

    assert!(
        matches!(
            error,
            ParseError::UnknownValue {
                tag: "initiator",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn the_optional_tags_may_be_absent() {
    // Both columns are nullable: a dispute is still a dispute without them,
    // and it still counts by status and instance.
    for tag in ["initiator", "created_at"] {
        let dispute = parse(&dispute_but(tag, None)).expect(tag);

        match tag {
            "initiator" => assert_eq!(dispute.initiator, None),
            _ => assert_eq!(dispute.opened_at, None),
        }
    }
}

#[test]
fn each_required_tag_is_named_when_it_is_missing() {
    for tag in ["d", "s"] {
        let error = parse(&dispute_but(tag, None)).expect_err(tag);

        assert_eq!(error, ParseError::MissingTag { tag }, "removing {tag}");
    }
}

#[test]
fn an_event_of_another_kind_is_rejected_before_any_tag_is_read() {
    let error = parse(&load(38383, "canceled")).expect_err("an order is not a dispute");

    assert_eq!(
        error,
        ParseError::WrongKind {
            expected: KIND,
            found: 38383,
        }
    );
}
