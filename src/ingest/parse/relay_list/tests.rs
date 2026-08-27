//! Parsing kind 10002, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag, Timestamp};

use super::*;
use crate::ingest::parse::fixtures::load;

const CREATED_AT: u64 = 1_787_740_773;

/// A relay-list event carrying `tags`, signed by a throwaway key.
fn relay_event(tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::from_u16(KIND), "")
        .tags(
            tags.iter()
                .map(|values| Tag::parse(values.iter().copied()).expect("well-formed tag")),
        )
        .custom_created_at(Timestamp::from_secs(CREATED_AT))
        .finalize(&Keys::generate())
        .expect("signing")
}

#[test]
fn the_captured_corpus_parses() {
    // Arrange
    let event = load(KIND, "typical");

    // Act
    let list = parse(&event).expect("a relay list");

    // Assert
    assert_eq!(
        list.relays,
        ["wss://nos.lol", "wss://relay.mostro.network"],
        "in the order published"
    );
    assert_eq!(list.pubkey, event.pubkey.to_hex());
    assert_eq!(list.created_at, event.created_at.as_secs() as i64);
}

#[test]
fn a_relay_the_instance_only_reads_from_is_not_where_it_publishes() {
    // NIP-65: no marker means both; `write` is where its events land;
    // `read` is where it listens, and nothing of its own is there.
    let event = relay_event(&[
        &["r", "wss://both.example"],
        &["r", "wss://writes.example", "write"],
        &["r", "wss://reads.example", "read"],
    ]);

    let list = parse(&event).expect("a relay list");

    assert_eq!(list.relays, ["wss://both.example", "wss://writes.example"]);
}

#[test]
fn a_marker_nip_65_does_not_define_is_no_claim_to_publish_there() {
    // NIP-65 spells three cases and no more. A fourth says nothing, and a
    // relay bestiario would dial has to be one an instance said it
    // publishes to — not merely one it did not say `read` about.
    let event = relay_event(&[
        &["r", "wss://typo.example", "wrote"],
        &["r", "wss://foreign.example", "foo"],
        &["r", "wss://shouting.example", "WRITE"],
        &["r", "wss://good.example", "write"],
    ]);

    let list = parse(&event).expect("a relay list");

    assert_eq!(
        list.relays,
        ["wss://good.example"],
        "only the marker the NIP defines carries the claim"
    );
}

#[test]
fn a_relay_url_is_normalised_the_way_the_client_will_dial_it() {
    let event = relay_event(&[
        &["r", "WSS://Relay.Example/"],
        &["r", "wss://relay.example"],
    ]);

    let list = parse(&event).expect("a relay list");

    assert_eq!(
        list.relays,
        ["wss://relay.example"],
        "the same relay twice is one relay"
    );
}

#[test]
fn a_url_that_is_not_a_relay_is_dropped_and_the_rest_kept() {
    // One malformed entry does not throw away an instance's whole list;
    // there is nothing to dial there and everything else still works.
    let event = relay_event(&[
        &["r", "not a url"],
        &["r", "https://relay.example"],
        &["r", "wss://good.example"],
    ]);

    let list = parse(&event).expect("a relay list");

    assert_eq!(list.relays, ["wss://good.example"]);
}

#[test]
fn an_empty_list_is_a_list_that_names_nowhere() {
    let event = relay_event(&[]);

    let list = parse(&event).expect("a relay list");

    assert!(list.relays.is_empty());
}

#[test]
fn an_r_tag_with_no_value_is_skipped() {
    let event = relay_event(&[&["r"], &["r", "wss://good.example"]]);

    let list = parse(&event).expect("a relay list");

    assert_eq!(list.relays, ["wss://good.example"]);
}

#[test]
fn another_kind_is_not_a_relay_list() {
    let event = EventBuilder::new(Kind::from_u16(1), "")
        .finalize(&Keys::generate())
        .expect("signing");

    assert!(matches!(
        parse(&event),
        Err(ParseError::WrongKind { expected: KIND, .. })
    ));
}
