//! Parsing kind 30078, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// A rate event with the given content and tags, signed by a throwaway key.
fn rates_event(content: &str, tags: &[(&str, &str)]) -> Event {
    EventBuilder::new(Kind::from_u16(KIND), content)
        .tags(
            tags.iter()
                .map(|(name, value)| Tag::parse([*name, *value]).expect("well-formed tag")),
        )
        .finalize(&Keys::generate())
        .expect("signing")
}

fn valid_tags() -> Vec<(&'static str, &'static str)> {
    vec![
        ("d", "mostro-rates"),
        ("published_at", "1787740773"),
        ("source", "yadio"),
    ]
}

#[test]
fn a_captured_snapshot_parses_with_its_source_and_time() {
    // Arrange / Act
    let snapshot = parse(&load(30078, "typical")).expect("parse");

    // Assert
    assert_eq!(snapshot.published_at, 1_787_740_773);
    assert_eq!(snapshot.source.as_deref(), Some("yadio"));
    assert!(snapshot.rates.len() > 100, "{}", snapshot.rates.len());
    assert!(snapshot.rates["USD"] > 0.0);
    assert!(snapshot.rates["ARS"] > snapshot.rates["USD"]);
}

#[test]
fn every_captured_snapshot_parses() {
    for name in ["typical", "another_instance"] {
        let snapshot = parse(&load(30078, name)).expect(name);
        assert!(snapshot.rates.contains_key("USD"), "{name}");
    }
}

#[test]
fn the_publication_time_falls_back_to_the_event_clock() {
    let event = rates_event(r#"{"BTC":{"USD":50000.0}}"#, &[("d", "mostro-rates")]);

    let snapshot = parse(&event).expect("parse");

    assert_eq!(snapshot.published_at, event.created_at.as_secs() as i64);
    assert_eq!(snapshot.source, None);
}

#[test]
fn a_snapshot_under_another_identifier_is_not_a_rate_snapshot() {
    let event = rates_event(r#"{"BTC":{"USD":1.0}}"#, &[("d", "something-else")]);

    let error = parse(&event).expect_err("wrong d");

    assert!(
        matches!(error, ParseError::UnknownValue { tag: "d", .. }),
        "{error}"
    );
}

#[test]
fn content_that_is_not_json_is_an_error() {
    let error = parse(&rates_event("not json", &valid_tags())).expect_err("bad content");

    assert!(
        matches!(error, ParseError::InvalidContent { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("not JSON"), "{error}");
}

#[test]
fn content_without_the_btc_table_is_an_error() {
    let error = parse(&rates_event(r#"{"ETH":{"USD":1.0}}"#, &valid_tags())).expect_err("no BTC");

    assert!(error.to_string().contains("`BTC`"), "{error}");
}

#[test]
fn a_rate_that_is_not_a_positive_number_is_an_error_rather_than_a_silent_skip() {
    for content in [
        r#"{"BTC":{"USD":"50000"}}"#,
        r#"{"BTC":{"USD":-1.0}}"#,
        r#"{"BTC":{"USD":0}}"#,
        r#"{"BTC":{"USD":null}}"#,
    ] {
        let error = parse(&rates_event(content, &valid_tags())).expect_err(content);
        assert!(
            matches!(error, ParseError::InvalidContent { .. }),
            "{content}: {error}"
        );
    }
}

#[test]
fn an_empty_table_is_an_error() {
    let error = parse(&rates_event(r#"{"BTC":{}}"#, &valid_tags())).expect_err("empty");

    assert!(error.to_string().contains("names no currency"), "{error}");
}

#[test]
fn an_unreadable_publication_time_is_an_error() {
    let event = rates_event(
        r#"{"BTC":{"USD":1.0}}"#,
        &[("d", "mostro-rates"), ("published_at", "yesterday")],
    );

    let error = parse(&event).expect_err("bad time");

    assert!(
        matches!(
            error,
            ParseError::NotANumber {
                tag: "published_at",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn the_wrong_kind_is_refused_before_anything_is_read() {
    let error = parse(&load(38385, "typical")).expect_err("wrong kind");

    assert_eq!(
        error,
        ParseError::WrongKind {
            expected: KIND,
            found: 38385
        }
    );
}
