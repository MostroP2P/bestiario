//! Guards on the fixture corpus itself.
//!
//! Every parser test in phase 1 reads these files, so a fixture that is
//! malformed, unsigned or filed under the wrong kind would not fail here — it
//! would fail somewhere else, much later, looking like a parser bug. These
//! tests keep that from happening.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nostr_sdk::prelude::*;

/// Kinds bestiario indexes, and the least number of fixtures each needs for
/// its parser to be tested against more than the happy path.
const EXPECTED: [(u16, usize); 6] = [
    (38383, 11),
    (8383, 2),
    (38386, 4),
    (38385, 4),
    (30078, 2),
    (10002, 2),
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every fixture, as `(relative path, parsed event)`.
fn load_all() -> Vec<(String, Event)> {
    let root = fixtures_dir();
    let mut found = Vec::new();

    for kind_dir in std::fs::read_dir(&root).expect("fixtures directory") {
        let kind_dir = kind_dir.expect("directory entry").path();
        if !kind_dir.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(&kind_dir).expect("kind directory") {
            let path = file.expect("directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .expect("path is under the fixtures root")
                .display()
                .to_string();
            let raw = std::fs::read_to_string(&path).expect("read fixture");
            let event = Event::from_json(&raw)
                .unwrap_or_else(|e| panic!("{relative} is not a valid Nostr event: {e}"));
            found.push((relative, event));
        }
    }

    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        !found.is_empty(),
        "no fixtures found under {}",
        root.display()
    );
    found
}

#[test]
fn every_fixture_carries_a_valid_signature() {
    // The pipeline verifies signatures before persisting anything (SPEC §8.1
    // step 2), so an unsigned or tampered fixture would be silently discarded
    // and its test would assert against an empty database.
    for (path, event) in load_all() {
        event
            .verify()
            .unwrap_or_else(|e| panic!("{path} has an invalid signature or id: {e}"));
    }
}

#[test]
fn every_fixture_sits_in_the_directory_named_after_its_kind() {
    for (path, event) in load_all() {
        let directory = path.split('/').next().expect("a kind directory");
        assert_eq!(
            directory,
            event.kind.as_u16().to_string(),
            "{path} holds a kind {} event",
            event.kind.as_u16()
        );
    }
}

#[test]
fn every_indexed_kind_has_enough_fixtures_to_test_more_than_the_happy_path() {
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for (_, event) in load_all() {
        *counts.entry(event.kind.as_u16()).or_default() += 1;
    }

    for (kind, minimum) in EXPECTED {
        let found = counts.get(&kind).copied().unwrap_or(0);
        assert!(
            found >= minimum,
            "kind {kind} has {found} fixtures, expected at least {minimum}"
        );
    }
}

#[test]
fn no_two_fixtures_are_the_same_event() {
    // Duplicates would make a test look broader than it is.
    let mut seen: BTreeMap<EventId, String> = BTreeMap::new();
    for (path, event) in load_all() {
        if let Some(first) = seen.insert(event.id, path.clone()) {
            panic!("{path} is the same event as {first}");
        }
    }
}

#[test]
fn the_order_corpus_covers_every_published_status() {
    // The daemon publishes exactly four statuses on the wire (SPEC §2.1), and
    // the projection logic in PR 13 branches on all of them.
    let mut statuses: Vec<String> = load_all()
        .into_iter()
        .filter(|(_, e)| e.kind.as_u16() == 38383)
        .filter_map(|(_, e)| tag_value(&e, "s"))
        .collect();
    statuses.sort();
    statuses.dedup();

    for expected in ["pending", "in-progress", "success", "canceled"] {
        assert!(
            statuses.iter().any(|s| s == expected),
            "no order fixture with status `{expected}`; have {statuses:?}"
        );
    }
}

#[test]
fn the_order_corpus_covers_the_awkward_shapes() {
    let orders: Vec<Event> = load_all()
        .into_iter()
        .map(|(_, e)| e)
        .filter(|e| e.kind.as_u16() == 38383)
        .collect();

    // A range order publishes `fa` with two values rather than one.
    assert!(
        orders.iter().any(|e| tag_values(e, "fa").len() == 2),
        "no range order fixture"
    );
    // A market-price order that has not been taken publishes `amt` of zero.
    assert!(
        orders
            .iter()
            .any(|e| tag_value(e, "amt").as_deref() == Some("0")),
        "no market-price order fixture"
    );
    // An order may list several payment methods in one `pm` tag.
    assert!(
        orders.iter().any(|e| tag_values(e, "pm").len() > 1),
        "no multi-payment-method fixture"
    );
}

#[test]
fn every_mostro_order_publishes_expires_at() {
    // Worth pinning down, because it is only true once platforms are told
    // apart. Across a 200-order sample every Mostro order carried
    // `expires_at` and every hodlhodl, telegram and Bitway order omitted it —
    // so a corpus that mixed platforms would suggest the field is optional
    // for Mostro, and the parser would be written to tolerate its absence.
    for (path, event) in load_all() {
        if event.kind.as_u16() != 38383 || tag_value(&event, "y").as_deref() != Some("mostro") {
            continue;
        }
        assert!(
            tag_value(&event, "expires_at").is_some(),
            "{path} is a Mostro order with no expires_at"
        );
    }
}

#[test]
fn the_corpus_includes_platforms_that_are_not_mostro() {
    // The first value of `y` is the platform, not always `mostro`: hodlhodl,
    // telegram bots and others publish NIP-69 orders to the same relays.
    // bestiario measures the Mostro network, so it has to be able to tell them
    // apart — and it cannot be tested on that without examples.
    let platforms: Vec<String> = load_all()
        .into_iter()
        .filter_map(|(_, e)| tag_value(&e, "y"))
        .collect();

    assert!(
        platforms.iter().any(|p| p == "mostro"),
        "no Mostro fixture; have {platforms:?}"
    );
    assert!(
        platforms.iter().filter(|p| *p != "mostro").count() >= 3,
        "expected at least three non-Mostro platform fixtures; have {platforms:?}"
    );
}

#[test]
fn the_corpus_includes_instances_that_publish_no_name() {
    // Nine of the twenty-two Mostro instances observed publish `y = ["mostro"]`
    // with no name at all, so reports have to cope with it.
    let nameless = load_all()
        .into_iter()
        .filter(|(_, e)| tag_value(e, "y").as_deref() == Some("mostro"))
        .filter(|(_, e)| tag_values(e, "y").len() == 1)
        .count();

    assert!(nameless >= 2, "expected fixtures for nameless instances");
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .find(|values| values.first().map(String::as_str) == Some(name))
        .map(|values| values[1..].to_vec())
        .unwrap_or_default()
}

fn tag_value(event: &Event, name: &str) -> Option<String> {
    tag_values(event, name).into_iter().next()
}
