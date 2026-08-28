//! The size gate of §9.1, over documents whose sizes are known.

use super::*;
use crate::bucket::Coverage;
use crate::publish::address::{Report, Window as Span};
use crate::publish::index::Publisher;
use crate::publish::snapshot::Snapshot;
use crate::series::Data;

const SEPTEMBER: i64 = 1_788_220_800;

fn snapshot() -> Snapshot {
    Snapshot::compute(
        &Data::default(),
        Coverage::since(SEPTEMBER - 86_400),
        "01J",
        SEPTEMBER,
    )
}

// ---- the ceiling

#[test]
fn a_relay_lowers_the_ceiling_and_never_raises_it() {
    let configured = Ceiling::configured(64 * 1024);

    let generous = configured.clone().and_relay("wss://big.example", 1_000_000);
    let strict = configured.clone().and_relay("wss://small.example", 8_192);

    assert_eq!(
        generous, configured,
        "a relay that would take more changes nothing"
    );
    assert_eq!(generous.relay(), None);
    assert_eq!(strict.bytes(), 8_192);
    assert_eq!(
        strict.relay(),
        Some("wss://small.example"),
        "the operator has to know which relay is the binding one"
    );
}

#[test]
fn the_smallest_relay_is_the_one_in_force() {
    let ceiling = Ceiling::configured(64 * 1024)
        .and_relay("wss://a.example", 32_768)
        .and_relay("wss://b.example", 16_384)
        .and_relay("wss://c.example", 65_536);

    assert_eq!(ceiling.bytes(), 16_384);
    assert_eq!(ceiling.relay(), Some("wss://b.example"));
}

#[test]
fn the_default_ceiling_is_conservative() {
    assert_eq!(DEFAULT_MAX_CONTENT_BYTES, 65_536);
}

// ---- weighing

#[test]
fn a_document_weighs_its_content_and_not_its_payload() {
    let snapshot = snapshot();
    let document = snapshot.documents.first().expect("a document");

    let measured = measure(&snapshot.documents);

    assert_eq!(measured.len(), snapshot.documents.len());
    assert_eq!(measured[0].address, document.address);
    assert_eq!(measured[0].hash.as_deref(), Some(document.hash.as_str()));
    assert_eq!(measured[0].bytes, document.content().len());
    assert!(
        measured[0].bytes > document.envelope.payload().to_string().len(),
        "the envelope is on the wire too, and the relay counts it"
    );
    assert!(
        document.content().contains("\"snapshot_id\":\"01J\""),
        "the content is the envelope, run and all: {}",
        document.content()
    );
}

// ---- the gate

#[test]
fn every_document_over_the_ceiling_is_named_not_just_the_first() {
    let measured = measure(&snapshot().documents);
    let smallest = measured
        .iter()
        .map(|document| document.bytes)
        .min()
        .expect("documents");

    // A ceiling under everything: an operator shrinking a snapshot wants
    // the whole list, not the next one on the next run.
    let over = over(&measured, &Ceiling::configured(smallest - 1));

    assert_eq!(over.len(), measured.len());
}

#[test]
fn a_document_exactly_at_the_ceiling_fits() {
    let measured = measure(&snapshot().documents);
    let largest = measured
        .iter()
        .map(|document| document.bytes)
        .max()
        .expect("documents");

    assert!(
        over(&measured, &Ceiling::configured(largest)).is_empty(),
        "the limit is a maximum length, not a length to stay under"
    );
    assert_eq!(over(&measured, &Ceiling::configured(largest - 1)).len(), 1);
}

#[test]
fn a_snapshot_that_fits_names_nothing() {
    let measured = measure(&snapshot().documents);

    assert!(over(&measured, &Ceiling::configured(DEFAULT_MAX_CONTENT_BYTES)).is_empty());
    // The addresses are carried through, so the error can name documents
    // rather than positions.
    assert!(measured.iter().any(|document| matches!(
        document.address,
        crate::publish::Address::Window {
            report: Report::Orders,
            window: Span::Hours24,
            ..
        }
    )));
}

#[test]
fn the_index_is_weighed_too_and_carries_no_hash() {
    // §9.1 weighs every document and §5.1 shards the index by year when
    // it approaches the limit, so an index nobody weighed is a snapshot
    // that fails at the relay rather than at the gate. Nothing hashes it
    // (§6), so there is no digest to carry.
    let snapshot = snapshot();
    let index = snapshot.index(&Publisher {
        name: "bestiario".to_string(),
        version: "0.4.0".to_string(),
    });

    let measured = measure_index(&index);

    assert_eq!(measured.address.to_string(), "index");
    assert_eq!(measured.bytes, index.content().len());
    assert!(measured.bytes > 0);
    assert_eq!(measured.hash, None);
    assert!(
        index.content().starts_with("{\"schema_version\":"),
        "the content is the whole index, with no envelope around it: {}",
        index.content()
    );
}
