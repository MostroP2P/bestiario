//! Filters as the relays will see them: every assertion is against the JSON
//! that goes on the wire, not against the builder's own accessors.

use serde_json::{Value, json};

use super::*;

const MOSTRO: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const OTHER: &str = "dbe0b1be7aafd3cfba92d7463edbd4e33b2969f61bd554d37ac56f032e13355c";
/// A half-open day: `[FROM, UNTIL)`.
const FROM: i64 = 1_787_700_000;
const UNTIL: i64 = 1_787_786_400;

fn pubkeys(hex: &[&str]) -> Vec<PublicKey> {
    hex.iter()
        .map(|hex| PublicKey::parse(hex).expect("valid pubkey"))
        .collect()
}

fn range() -> Range {
    Range::resolve(Some(FROM), Some(UNTIL), UNTIL).expect("valid range")
}

fn as_json(filter: &Filter) -> Value {
    serde_json::to_value(filter).expect("filters serialize")
}

#[test]
fn a_filter_names_its_kind_and_nothing_else_by_default() {
    // Arrange
    let authors: Vec<PublicKey> = Vec::new();

    // Act
    let filter = for_kind(order::KIND, &authors, None, None);

    // Assert
    assert_eq!(as_json(&filter), json!({ "kinds": [38383] }));
}

#[test]
fn an_empty_author_list_is_left_off_rather_than_sent_empty() {
    // `accept_unknown_instances = true` follows every publisher, and the
    // platform filter of SPEC 8.1 step 4 decides afterwards. A filter with an
    // empty `authors` array would instead match nothing at all.
    let filter = for_kind(order::KIND, &[], None, None);

    assert_eq!(as_json(&filter).get("authors"), None);
}

#[test]
fn the_configured_instances_become_the_author_set() {
    let filter = for_kind(order::KIND, &pubkeys(&[MOSTRO, OTHER]), None, None);

    // The wire form is a set, so the order relays receive is not meaningful.
    let json = as_json(&filter);
    let mut authors: Vec<&str> = json["authors"]
        .as_array()
        .expect("authors")
        .iter()
        .map(|author| author.as_str().expect("hex"))
        .collect();
    authors.sort_unstable();
    let mut expected = vec![MOSTRO, OTHER];
    expected.sort_unstable();
    assert_eq!(authors, expected);
}

#[test]
fn a_range_becomes_an_inclusive_window_one_second_shorter() {
    // Range is half-open so that consecutive windows tile; a Nostr filter is
    // inclusive at both ends. Passing `until` through unchanged would make two
    // adjacent backfill windows both fetch the event on the boundary.
    let filter = for_kind(order::KIND, &[], Some(range()), None);

    let json = as_json(&filter);
    assert_eq!(json["since"], json!(FROM));
    assert_eq!(json["until"], json!(UNTIL - 1));
}

#[test]
fn an_unbounded_range_sends_no_bounds_rather_than_a_sentinel() {
    // Range::unbounded is `[0, i64::MAX)`. Converted arithmetically that would
    // put `until = 9223372036854775806` on the wire — a timestamp no relay can
    // mean anything by, and the opposite of what "no upper bound" says. An
    // open end is an absent field in a Nostr filter.
    let filter = for_kind(order::KIND, &[], Some(Range::unbounded()), None);

    let json = as_json(&filter);
    assert_eq!(json.get("since"), None);
    assert_eq!(json.get("until"), None);
    assert_eq!(json, json!({ "kinds": [38383] }));
}

#[test]
fn no_range_sends_no_bounds() {
    let filter = for_kind(order::KIND, &[], None, None);

    let json = as_json(&filter);
    assert_eq!(json.get("since"), None);
    assert_eq!(json.get("until"), None);
}

#[test]
fn a_limit_is_sent_only_when_asked_for() {
    let bounded = for_kind(order::KIND, &[], None, Some(500));
    let unbounded = for_kind(order::KIND, &[], None, None);

    assert_eq!(as_json(&bounded)["limit"], json!(500));
    assert_eq!(as_json(&unbounded).get("limit"), None);
}

#[test]
fn every_field_travels_together() {
    let filter = for_kind(dev_fee::KIND, &pubkeys(&[MOSTRO]), Some(range()), Some(100));

    assert_eq!(
        as_json(&filter),
        json!({
            "kinds": [8383],
            "authors": [MOSTRO],
            "since": FROM,
            "until": UNTIL - 1,
            "limit": 100,
        })
    );
}

#[test]
fn there_is_one_filter_per_indexed_kind_in_order() {
    // One filter per kind, not one filter listing four kinds: the resume
    // cursor is per (relay, kind), and a shared filter would have to use the
    // oldest of the four and re-read what the others had covered.
    let filters = per_kind(&[], None, None);

    let kinds: Vec<Value> = filters
        .iter()
        .map(|filter| as_json(filter)["kinds"].clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            json!([38383]),
            json!([8383]),
            json!([38386]),
            json!([38385])
        ]
    );
}

#[test]
fn the_indexed_kinds_are_the_ones_with_a_parser() {
    // Kinds 30078 and 10002 are in the spec but have no parser yet.
    assert_eq!(
        INDEXED_KINDS,
        [order::KIND, dev_fee::KIND, dispute::KIND, info::KIND]
    );
    assert!(!INDEXED_KINDS.contains(&30078));
    assert!(!INDEXED_KINDS.contains(&10002));
}

#[test]
fn every_filter_of_a_batch_carries_the_same_window_and_authors() {
    let filters = per_kind(&pubkeys(&[MOSTRO]), Some(range()), Some(10));

    for filter in &filters {
        let json = as_json(filter);
        assert_eq!(json["authors"], json!([MOSTRO]));
        assert_eq!(json["since"], json!(FROM));
        assert_eq!(json["until"], json!(UNTIL - 1));
        assert_eq!(json["limit"], json!(10));
    }
}
