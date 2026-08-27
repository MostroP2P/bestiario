//! Parsing kind 30078, against the captured corpus and against events that
//! were deliberately broken.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag, Timestamp};

use super::*;
use crate::ingest::parse::fixtures::load;

/// A rate event with the given content and tags, signed by a throwaway key.
///
/// The event clock is set to whatever `published_at` claims, so that a test
/// about the content is not tripped by the chronology rule. Tests about that
/// rule use [`rates_event_signed_at`] to make the two clocks disagree.
fn rates_event(content: &str, tags: &[(&str, &str)]) -> Event {
    let claimed = tags
        .iter()
        .find(|(name, _)| *name == "published_at")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .unwrap_or(PUBLISHED_AT as u64);

    rates_event_signed_at(content, tags, claimed)
}

/// The same, signed at `created_at` whatever the tags claim.
fn rates_event_signed_at(content: &str, tags: &[(&str, &str)], created_at: u64) -> Event {
    EventBuilder::new(Kind::from_u16(KIND), content)
        .tags(
            tags.iter()
                .map(|(name, value)| Tag::parse([*name, *value]).expect("well-formed tag")),
        )
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(&Keys::generate())
        .expect("signing")
}

/// The second the captured corpus was published at.
const PUBLISHED_AT: i64 = 1_787_740_773;

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

#[test]
fn a_publication_time_before_the_epoch_is_an_error() {
    // The phase-3 lookup picks the newest snapshot at or before an order's
    // timestamp. A snapshot dated before the epoch would sort below every
    // order, so it could end up the only quote on offer and be reported with
    // an age of decades. `expires_at` on orders is guarded the same way.
    let event = rates_event(
        r#"{"BTC":{"USD":1.0}}"#,
        &[("d", "mostro-rates"), ("published_at", "-1")],
    );

    let error = parse(&event).expect_err("negative time");

    assert!(
        matches!(
            error,
            ParseError::OutOfRange {
                tag: "published_at",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_publication_time_far_behind_the_event_clock_is_an_error() {
    // 30078 is addressable, so an instance can replace its own snapshot at
    // will. Were `published_at` free of the signed clock, a snapshot signed
    // today could claim last month and rewrite a period already reported.
    let event = rates_event_signed_at(
        r#"{"BTC":{"USD":1.0}}"#,
        &[("d", "mostro-rates"), ("published_at", "1000")],
        2000,
    );

    let error = parse(&event).expect_err("backdated");

    assert!(
        matches!(
            error,
            ParseError::OutOfRange {
                tag: "published_at",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_publication_time_ahead_of_the_event_clock_is_an_error() {
    // Rates cannot be fetched after the event quoting them was signed.
    let ahead = (PUBLISHED_AT + MAX_CLOCK_DIVERGENCE_SECS + 1).to_string();
    let event = rates_event_signed_at(
        r#"{"BTC":{"USD":1.0}}"#,
        &[("d", "mostro-rates"), ("published_at", &ahead)],
        PUBLISHED_AT as u64,
    );

    let error = parse(&event).expect_err("post-dated");

    assert!(
        matches!(
            error,
            ParseError::OutOfRange {
                tag: "published_at",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_publication_time_within_the_tolerated_divergence_is_accepted() {
    // The captured producer sets both clocks to the same second; the window
    // exists to absorb a slow publish, not to license a backdate.
    let lagging = (PUBLISHED_AT - MAX_CLOCK_DIVERGENCE_SECS).to_string();
    let event = rates_event_signed_at(
        r#"{"BTC":{"USD":1.0}}"#,
        &[("d", "mostro-rates"), ("published_at", &lagging)],
        PUBLISHED_AT as u64,
    );

    let snapshot = parse(&event).expect("within the window");

    assert_eq!(
        snapshot.published_at,
        PUBLISHED_AT - MAX_CLOCK_DIVERGENCE_SECS
    );
}

#[test]
fn a_currency_repeated_in_the_table_is_an_error_rather_than_a_last_one_wins() {
    // `serde_json::Value` keeps the last of two equal members, so the price
    // stored would depend on member order and two consumers of the same
    // signed payload could disagree about it.
    let error = parse(&rates_event(
        r#"{"BTC":{"USD":50000.0,"USD":1.0}}"#,
        &valid_tags(),
    ))
    .expect_err("duplicate currency");

    assert!(error.to_string().contains("USD"), "{error}");
    assert!(error.to_string().contains("more than once"), "{error}");
}

#[test]
fn a_repeated_btc_table_is_an_error() {
    let error = parse(&rates_event(
        r#"{"BTC":{"USD":50000.0},"BTC":{"USD":1.0}}"#,
        &valid_tags(),
    ))
    .expect_err("duplicate BTC");

    assert!(error.to_string().contains("more than once"), "{error}");
}

#[test]
fn a_currency_code_that_is_not_three_uppercase_letters_is_an_error() {
    // `usd` and `USD ` would be stored as currencies of their own and never
    // match the `USD` an order is denominated in, reporting a missing rate
    // for a rate that was published. Both captured snapshots use exactly
    // three uppercase ASCII letters, for all 141 currencies.
    for code in ["usd", "USD ", " USD", "US", "USDT", "US\u{1}"] {
        let content = format!(r#"{{"BTC":{{"{code}":1.0}}}}"#);
        let error = parse(&rates_event(&content, &valid_tags())).expect_err(code);

        assert!(
            matches!(error, ParseError::InvalidContent { .. }),
            "{code}: {error}"
        );
    }
}

#[test]
fn a_clock_beyond_a_signed_second_count_is_rejected_rather_than_narrowed() {
    // Arrange: a clock past `i64::MAX`, which `as i64` would wrap into a
    // negative second and every later subtraction would overflow on.
    let event = rates_event_signed_at(
        r#"{"BTC":{"USD":50000.0}}"#,
        &[("d", "mostro-rates")],
        u64::MAX,
    );

    // Act
    let error = parse(&event).expect_err("no such second");

    // Assert
    assert!(
        matches!(
            error,
            ParseError::OutOfRange {
                tag: "created_at",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn content_that_is_not_an_object_says_what_was_expected() {
    // The message is the only thing that tells an operator why a snapshot
    // was turned away, so it names the shape rather than the type.
    let event = rates_event("[]", &valid_tags());

    let error = parse(&event).expect_err("refused");

    let message = error.to_string();
    assert!(
        message.contains("an object with a `BTC` table of currency → price"),
        "{message}"
    );
}

#[test]
fn a_btc_table_that_is_not_an_object_says_what_was_expected() {
    let event = rates_event(r#"{"BTC": 78614.25}"#, &valid_tags());

    let error = parse(&event).expect_err("refused");

    let message = error.to_string();
    assert!(
        message.contains("`BTC` to be an object of currency → price"),
        "{message}"
    );
}
