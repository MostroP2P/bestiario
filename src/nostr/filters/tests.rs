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

/// The filter for `kind`, with somebody vouched for.
///
/// Most of these tests are about the narrowing every kind shares, and an
/// untagged kind is only asked for at all once a publisher is vouched for;
/// the tests that are about *that* rule call [`for_kind`] directly.
fn filter(kind: u16, authors: &[PublicKey], range: Option<Range>, limit: Option<usize>) -> Filter {
    for_kind(kind, authors, &pubkeys(&[MOSTRO]), range, limit).expect("somebody is vouched for")
}

/// The same for a whole batch.
fn filters(authors: &[PublicKey], range: Option<Range>, limit: Option<usize>) -> Vec<Filter> {
    per_kind(authors, &pubkeys(&[MOSTRO]), range, limit)
}

#[test]
fn a_filter_names_its_kind_and_nothing_else_by_default() {
    // Arrange
    let authors: Vec<PublicKey> = Vec::new();

    // Act
    let filter = filter(order::KIND, &authors, None, None);

    // Assert
    assert_eq!(as_json(&filter), json!({ "kinds": [38383] }));
}

#[test]
fn an_empty_author_list_is_left_off_rather_than_sent_empty() {
    // `accept_unknown_instances = true` follows every publisher, and the
    // platform filter of SPEC 8.1 step 4 decides afterwards. A filter with an
    // empty `authors` array would instead match nothing at all.
    let filter = filter(order::KIND, &[], None, None);

    assert_eq!(as_json(&filter).get("authors"), None);
}

#[test]
fn the_configured_instances_become_the_author_set() {
    let filter = filter(order::KIND, &pubkeys(&[MOSTRO, OTHER]), None, None);

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
    let filter = filter(order::KIND, &[], Some(range()), None);

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
    let filter = filter(order::KIND, &[], Some(Range::unbounded()), None);

    let json = as_json(&filter);
    assert_eq!(json.get("since"), None);
    assert_eq!(json.get("until"), None);
    assert_eq!(json, json!({ "kinds": [38383] }));
}

#[test]
fn no_range_sends_no_bounds() {
    let filter = filter(order::KIND, &[], None, None);

    let json = as_json(&filter);
    assert_eq!(json.get("since"), None);
    assert_eq!(json.get("until"), None);
}

#[test]
fn a_limit_is_sent_only_when_asked_for() {
    let bounded = filter(order::KIND, &[], None, Some(500));
    let unbounded = filter(order::KIND, &[], None, None);

    assert_eq!(as_json(&bounded)["limit"], json!(500));
    assert_eq!(as_json(&unbounded).get("limit"), None);
}

#[test]
fn every_field_travels_together() {
    let filter = filter(dev_fee::KIND, &pubkeys(&[MOSTRO]), Some(range()), Some(100));

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
    let filters = filters(&[], None, None);

    let kinds: Vec<Value> = filters
        .iter()
        .map(|built| as_json(built)["kinds"].clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            json!([38383]),
            json!([8383]),
            json!([38386]),
            json!([38385]),
            json!([30078]),
            json!([10002])
        ]
    );
}

#[test]
fn the_indexed_kinds_are_the_ones_with_a_parser() {
    assert_eq!(
        INDEXED_KINDS,
        [
            order::KIND,
            dev_fee::KIND,
            dispute::KIND,
            info::KIND,
            rates::KIND,
            relay_list::KIND,
        ]
    );
}

#[test]
fn the_kinds_that_carry_no_platform_tag_are_walked_last() {
    // Their publisher is vouched for by having already been seen as an
    // instance, which the tagged kinds are what establish.
    let untagged = [rates::KIND, relay_list::KIND];
    let first_untagged = INDEXED_KINDS
        .iter()
        .position(|kind| untagged.contains(kind))
        .expect("both are indexed");

    assert!(
        INDEXED_KINDS[first_untagged..]
            .iter()
            .all(|kind| untagged.contains(kind)),
        "nothing tagged follows an untagged kind"
    );
}

#[test]
fn every_filter_of_a_batch_carries_the_same_window_and_authors() {
    let filters = filters(&pubkeys(&[MOSTRO]), Some(range()), Some(10));

    for built in &filters {
        let json = as_json(built);
        assert_eq!(json["authors"], json!([MOSTRO]));
        assert_eq!(json["since"], json!(FROM));
        assert_eq!(json["until"], json!(UNTIL - 1));
        assert_eq!(json["limit"], json!(10));
    }
}

#[test]
fn the_rate_filter_names_the_mostro_identifier() {
    // Kind 30078 is NIP-78's generic application-data kind: every app on the
    // relay stores under it. Without the `d` bound, backfill and sync would
    // download every unrelated 30078 address to archive it as a rejection.
    // The authors are the vouched set, since 30078 is untagged: an empty
    // `authors` here is what the helper's vouched list becomes.
    let filter = filter(rates::KIND, &[], None, None);

    assert_eq!(
        as_json(&filter),
        json!({ "kinds": [30078], "#d": ["mostro-rates"], "authors": [MOSTRO] })
    );
}

#[test]
fn the_rate_identifier_survives_the_author_and_range_narrowing() {
    let filter = filter(rates::KIND, &pubkeys(&[MOSTRO]), Some(range()), Some(10));

    assert_eq!(as_json(&filter)["#d"], json!(["mostro-rates"]));
}

#[test]
fn only_the_rate_filter_carries_an_identifier() {
    // The other kinds are Mostro's own, so their kind number already says
    // what they are; a `d` bound there would only narrow them wrongly.
    for kind in INDEXED_KINDS.into_iter().filter(|&k| k != rates::KIND) {
        assert_eq!(as_json(&filter(kind, &[], None, None)).get("#d"), None);
    }
}

#[test]
fn every_filter_of_the_set_is_the_one_its_kind_would_get_alone() {
    // per_kind is the production path; this pins that it does not lose the
    // specialization for_kind applies.
    let per_kind = filters(&[], None, None);

    for (built, kind) in per_kind.iter().zip(INDEXED_KINDS) {
        assert_eq!(as_json(built), as_json(&filter(kind, &[], None, None)));
    }
}

#[test]
fn an_untagged_kind_is_asked_of_the_vouched_authors_even_when_any_author_would_do() {
    // `accept_unknown_instances = true` empties the author list, which for a
    // tagged kind means "anybody". For kind 10002 it would mean every NIP-65
    // relay list on the relay — the whole network's — every one of which
    // §8.1 step 4b would throw away after downloading and verifying it.
    for kind in [rates::KIND, relay_list::KIND] {
        let filter =
            for_kind(kind, &[], &pubkeys(&[MOSTRO]), None, None).expect("somebody is vouched for");

        assert_eq!(as_json(&filter)["authors"], json!([MOSTRO]), "kind {kind}");
    }
}

#[test]
fn the_vouched_set_overrides_the_author_set_for_an_untagged_kind() {
    // Not the union: a publisher listed for the tagged kinds is not thereby
    // vouched for an untagged one, which is step 4b's whole point.
    let filter = for_kind(
        relay_list::KIND,
        &pubkeys(&[OTHER]),
        &pubkeys(&[MOSTRO]),
        None,
        None,
    )
    .expect("somebody is vouched for");

    assert_eq!(as_json(&filter)["authors"], json!([MOSTRO]));
}

#[test]
fn an_untagged_kind_with_nobody_vouched_is_not_asked_for_at_all() {
    // The alternative would be a filter with no `authors`, which is the
    // global crawl; an empty `authors` array matches nothing, which is a
    // request with no purpose. Not asking is the only honest third option.
    for kind in [rates::KIND, relay_list::KIND] {
        assert!(
            for_kind(kind, &[], &[], None, None).is_none(),
            "kind {kind} was asked for with nobody to ask about"
        );
        assert!(
            for_kind(kind, &pubkeys(&[MOSTRO]), &[], None, None).is_none(),
            "kind {kind}: an author set is not a vouching"
        );
    }
}

#[test]
fn a_tagged_kind_is_asked_for_whether_or_not_anybody_is_vouched() {
    // The `y` tag is what vouches for these, and it travels with the event.
    for kind in INDEXED_KINDS
        .into_iter()
        .filter(|kind| !UNTAGGED_KINDS.contains(kind))
    {
        let filter = for_kind(kind, &[], &[], None, None).expect("always asked for");
        assert_eq!(as_json(&filter).get("authors"), None, "kind {kind}");
    }
}

#[test]
fn a_batch_with_nobody_vouched_is_the_tagged_kinds_alone() {
    let batch = per_kind(&[], &[], None, None);

    let kinds: Vec<u64> = batch
        .iter()
        .map(|built| as_json(built)["kinds"][0].as_u64().expect("a kind"))
        .collect();
    assert_eq!(kinds, vec![38383, 8383, 38386, 38385]);
    assert_eq!(
        batch.len(),
        INDEXED_KINDS.len() - UNTAGGED_KINDS.len(),
        "the untagged kinds are left out, not sent unnarrowed"
    );
}
