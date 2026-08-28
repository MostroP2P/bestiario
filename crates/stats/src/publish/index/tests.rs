//! The index as §5 describes it, over a snapshot of a hand-built archive.

use super::*;
use crate::activity::{Direction, Order, Origin, Status};
use crate::bucket::Coverage;
use crate::publish::address::Address;
use crate::publish::snapshot::Snapshot;
use crate::series::Data;

/// 2026-07-01, 2026-08-01 and 2026-09-01 at midnight UTC.
const JULY: i64 = 1_782_864_000;
const AUGUST: i64 = 1_785_542_400;
const SEPTEMBER: i64 = 1_788_220_800;
const DAY: i64 = 86_400;

fn order(id: &str, created_at: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at,
        status: Status::Success,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec!["cash".into()],
        amount_sats: 10_000,
        fiat_amount: Some(50.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(created_at),
        origin: Origin {
            fiat_code: "ARS".into(),
            payment_methods: vec!["cash".into()],
            direction: Direction::Buy,
        },
        taken_at: Some(created_at + 60),
        success_at: Some(created_at + DAY),
        canceled_at: None,
        expires_at: Some(created_at + 2 * DAY),
    }
}

fn data() -> Data {
    Data {
        orders: vec![order("j1", JULY + DAY), order("a1", AUGUST + DAY)],
        ..Data::default()
    }
}

fn coverage() -> Coverage {
    Coverage::between(JULY, AUGUST + 2 * DAY)
}

fn publisher() -> Publisher {
    Publisher {
        name: "bestiario".to_string(),
        version: "0.4.0".to_string(),
    }
}

fn snapshot() -> Snapshot {
    Snapshot::compute(&data(), coverage(), "01J8Z", SEPTEMBER)
}

fn payload_of(snapshot: &Snapshot) -> serde_json::Value {
    snapshot.index(&publisher()).envelope.payload().clone()
}

// ---- what exists (§5.1)

#[test]
fn the_index_lists_every_document_of_the_snapshot_and_never_itself() {
    let snapshot = snapshot();

    let json = payload_of(&snapshot);
    let listed: Vec<String> = json["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|entry| entry["d"].as_str().expect("d").to_string())
        .collect();

    assert_eq!(
        listed.len(),
        snapshot.documents.len(),
        "one entry per document, no more and no fewer"
    );
    assert!(
        !listed.iter().any(|d| d == "index"),
        "the index is how a client finds the rest; it does not find itself"
    );
    for document in &snapshot.documents {
        assert!(
            listed.contains(&document.address.to_string()),
            "{} is missing from {listed:?}",
            document.address
        );
    }
}

#[test]
fn the_index_is_addressed_as_index_and_carries_the_runs_clock() {
    let index = snapshot().index(&publisher());

    assert_eq!(index.address, Address::Index { year: None });
    assert_eq!(index.envelope.snapshot_id(), "01J8Z");
    assert_eq!(index.envelope.revision(), 1);
}

// ---- what changed (§5.2)

#[test]
fn an_entry_carries_the_hash_of_the_payload_that_document_published() {
    let snapshot = snapshot();

    let json = payload_of(&snapshot);

    for (entry, document) in json["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .zip(&snapshot.documents)
    {
        assert_eq!(
            entry["hash"], document.hash,
            "{}: the index quotes the document's own hash rather than hashing it again",
            document.address
        );
        assert_eq!(entry["revision"], 1);
        assert_eq!(entry["updated_at"], "2026-09-01T00:00:00+00:00");
        assert!(
            entry.get("restated_at").is_none(),
            "a first revision restates nothing"
        );
        assert!(entry.get("restated_because").is_none());
    }
}

#[test]
fn the_hash_is_over_the_figures_and_not_over_the_run_around_them() {
    let snapshot = snapshot();
    let document = snapshot.documents.first().expect("a document");

    let json = payload_of(&snapshot);

    // The same figures published by a later run: a new `snapshot_id` and
    // a new clock, and the hash a client compares must not move.
    let again = Snapshot::compute(&data(), coverage(), "01J9A", SEPTEMBER);
    let later = again.index(&publisher()).envelope.payload().clone();

    assert_eq!(json["documents"][0]["hash"], document.hash);
    assert_eq!(
        later["documents"][0]["hash"], json["documents"][0]["hash"],
        "a hash over the content would make every closed partition a new revision every run"
    );
}

// ---- the archive's real extent

#[test]
fn the_index_states_where_the_archive_begins_and_ends() {
    let json = payload_of(&snapshot());

    assert_eq!(
        json["coverage"]["first_event_at"],
        "2026-07-01T00:00:00+00:00"
    );
    assert_eq!(
        json["coverage"]["last_event_at"],
        "2026-08-03T00:00:00+00:00"
    );
    assert_eq!(json["publisher"]["name"], "bestiario");
    assert_eq!(json["publisher"]["version"], "0.4.0");
}

#[test]
fn an_archive_that_holds_nothing_states_that_rather_than_a_date() {
    let empty = Snapshot::compute(&Data::default(), Coverage::default(), "01J8Z", SEPTEMBER);

    let json = empty.index(&publisher()).envelope.payload().clone();

    assert!(json["coverage"]["first_event_at"].is_null());
    assert!(json["coverage"]["last_event_at"].is_null());
    assert!(
        json["resolutions"]
            .as_object()
            .expect("resolutions")
            .is_empty(),
        "no partition was published, so there is no resolution to pick from"
    );
    // The window documents are still there, and still say what they
    // found: nothing. It is the `coverage` block above that tells a
    // client not to read those zeros as a quiet market (§5), which is
    // why an empty extent has to be stated rather than omitted.
    let listed: Vec<&str> = json["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|entry| entry["d"].as_str().expect("d"))
        .collect();
    assert!(!listed.is_empty());
    assert!(
        !listed.iter().any(|d| d.starts_with("series:")),
        "an archive holding nothing has no partition to publish: {listed:?}"
    );
}

// ---- the resolutions a client picks from

#[test]
fn each_resolution_runs_from_its_first_published_partition_to_its_last() {
    let json = payload_of(&snapshot());
    let resolutions = json["resolutions"].as_object().expect("resolutions");

    assert_eq!(resolutions["daily"]["from"], "2026-07");
    assert_eq!(resolutions["daily"]["until"], "2026-09");
    assert_eq!(resolutions["weekly"]["from"], "2026-07");
    assert_eq!(resolutions["monthly"]["from"], "2026");
    assert_eq!(resolutions["monthly"]["until"], "2026");
}

#[test]
fn a_resolution_nothing_was_published_at_is_not_offered() {
    // An archive whose every partition falls outside coverage publishes
    // no series at all, and an index that still advertised `daily` would
    // send every client after a document that does not exist.
    let json = Snapshot::compute(&Data::default(), Coverage::default(), "01J8Z", SEPTEMBER)
        .index(&publisher())
        .envelope
        .payload()
        .clone();

    assert!(json["resolutions"].get("daily").is_none());
}

// ---- determinism

#[test]
fn the_same_snapshot_indexes_to_the_same_bytes() {
    let one = serde_json::to_vec(snapshot().index(&publisher()).envelope.payload()).expect("bytes");
    let two = serde_json::to_vec(snapshot().index(&publisher()).envelope.payload()).expect("bytes");

    assert_eq!(
        one, two,
        "the resolutions are ordered, not hashed into a map"
    );
    let text = String::from_utf8(one).expect("utf-8");
    assert!(
        text.find("\"daily\"") < text.find("\"monthly\""),
        "a stable order, whichever it is: {text}"
    );
}
