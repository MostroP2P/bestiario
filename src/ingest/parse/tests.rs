//! The tag helpers every parser is built from, against the shapes a relay
//! can actually deliver: a tag with no value at all, and one with more
//! values than the question has answers.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;

/// An event carrying `tags`, signed by a throwaway key. The kind does not
/// matter: these helpers read tags and nothing else.
fn tagged(tags: &[Vec<&str>]) -> Event {
    EventBuilder::new(Kind::from_u16(1), "")
        .tags(
            tags.iter()
                .map(|values| Tag::parse(values.iter().copied()).expect("well-formed tag")),
        )
        .finalize(&Keys::generate())
        .expect("signing")
}

#[test]
fn a_required_tag_published_with_no_value_answers_nothing() {
    // `["f"]` — the tag is there and says nothing. Storing that would open
    // a currency bucket named after the empty string.
    let event = tagged(&[vec!["f"]]);

    let error = required(&event, "f").expect_err("refused");

    assert!(
        matches!(error, ParseError::EmptyTag { tag: "f" }),
        "{error}"
    );
}

#[test]
fn a_required_tag_with_several_values_has_no_single_answer() {
    let event = tagged(&[vec!["f", "ARS", "USD"]]);

    let error = required(&event, "f").expect_err("refused");

    assert!(
        matches!(
            error,
            ParseError::WrongValueCount {
                tag: "f",
                count: 2,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_required_tag_that_is_absent_says_so() {
    let event = tagged(&[]);

    let error = required(&event, "f").expect_err("refused");

    assert!(
        matches!(error, ParseError::MissingTag { tag: "f" }),
        "{error}"
    );
}

#[test]
fn an_optional_tag_published_with_no_value_reads_as_absent() {
    // Real instances publish `["lnd_uris"]` and `["fiat_currencies_accepted", ""]`;
    // neither is an answer worth storing apart from having published none.
    let empty = tagged(&[vec!["lnd_uris"]]);
    let blank = tagged(&[vec!["lnd_uris", "   "]]);

    assert_eq!(optional(&empty, "lnd_uris").expect("read"), None);
    assert_eq!(optional(&blank, "lnd_uris").expect("read"), None);
    assert_eq!(optional(&tagged(&[]), "lnd_uris").expect("read"), None);
}

#[test]
fn an_optional_tag_with_several_values_is_still_refused() {
    // Optional is about whether the question was answered, not about
    // accepting two answers to it.
    let event = tagged(&[vec!["source", "yadio", "coingecko"]]);

    let error = optional(&event, "source").expect_err("refused");

    assert!(
        matches!(
            error,
            ParseError::WrongValueCount {
                tag: "source",
                count: 2,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_tag_published_twice_is_two_answers_to_one_question() {
    let event = tagged(&[vec!["d", "first"], vec!["d", "second"]]);

    let error = required(&event, "d").expect_err("refused");

    assert!(
        matches!(error, ParseError::RepeatedTag { tag: "d", count: 2 }),
        "{error}"
    );
}
