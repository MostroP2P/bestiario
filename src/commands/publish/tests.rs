//! The decisions `publish` makes on its own: the run's identity, the
//! ceiling it refuses against, and what a review prints.

use super::*;
use crate::stats::series::Data;

/// 2026-08-27T03:06:40Z, the clock the E2E suite freezes.
const NOW: i64 = 1_787_800_000;

fn publication(coverage: Coverage, ceiling: Ceiling) -> Publication {
    let snapshot = Snapshot::compute(&Data::default(), coverage, &snapshot_id(NOW), NOW);
    let index = snapshot.index(&publisher());
    let measured = size::measure(
        &snapshot
            .documents
            .iter()
            .cloned()
            .chain(std::iter::once(index.clone()))
            .collect::<Vec<_>>(),
    );

    Publication {
        snapshot,
        index,
        ceiling,
        relays_asked: 1,
        measured,
    }
}

// ---- the run's identity (§7)

#[test]
fn a_snapshot_id_is_the_runs_clock_and_two_runs_sort_in_order() {
    let earlier = snapshot_id(NOW);
    let later = snapshot_id(NOW + 1);

    assert_eq!(earlier, "20260827T030640Z");
    assert!(
        earlier < later,
        "monotonic as text as well as in time: {earlier} then {later}"
    );
}

#[test]
fn a_clock_no_date_can_hold_still_names_the_run() {
    // Falling back to the number keeps a run identifiable rather than
    // panicking on a clock nobody should have set.
    assert_eq!(snapshot_id(i64::MAX), i64::MAX.to_string());
}

// ---- the ceiling (§9.1)

#[test]
fn a_document_over_the_ceiling_is_named_and_the_ceiling_is_explained() {
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(100));

    let refusal = refuse_oversized(&publication).expect_err("everything is over 100 bytes");

    let message = refusal.to_string();
    assert!(message.contains("100-byte ceiling"), "{message}");
    assert!(
        message.contains("[publish].max_content_bytes"),
        "the operator has to know which ceiling to raise: {message}"
    );
    assert!(message.contains("orders:24h is "), "{message}");
    assert!(
        message.contains("index is "),
        "the index is weighed like everything else: {message}"
    );
}

#[test]
fn a_relay_that_set_the_ceiling_is_the_one_named() {
    let publication = publication(
        Coverage::since(NOW - 86_400),
        Ceiling::configured(65_536).and_relay("wss://strict.example", 100),
    );

    let message = refuse_oversized(&publication)
        .expect_err("over the relay's limit")
        .to_string();

    assert!(
        message.contains("advertised by wss://strict.example"),
        "{message}"
    );
}

#[test]
fn a_snapshot_that_fits_is_not_refused() {
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));

    assert!(refuse_oversized(&publication).is_ok());
}

// ---- the listing

#[test]
fn a_listing_names_the_run_the_extent_and_the_ceiling() {
    let publication = publication(
        Coverage::between(NOW - 86_400, NOW - 60),
        Ceiling::configured(65_536),
    );

    let listing = listing(&publication);

    assert!(
        listing.starts_with("snapshot 20260827T030640Z generated at 2026-08-27T03:06:40+00:00\n")
    );
    assert!(
        listing.contains("archive 2026-08-26T03:06:40+00:00 to 2026-08-27T03:05:40+00:00\n"),
        "{listing}"
    );
    assert!(
        listing.contains("ceiling 65536 bytes ([publish].max_content_bytes), 1 relay(s) asked"),
        "{listing}"
    );
}

#[test]
fn an_empty_archive_says_so_rather_than_printing_two_blanks() {
    // The window documents below the line are full of zeros; this is what
    // says why (§5), and a reader who misses it reads a quiet market.
    let listing = listing(&publication(
        Coverage::default(),
        Ceiling::configured(65_536),
    ));

    assert!(listing.contains("archive holds nothing\n"), "{listing}");
}

#[test]
fn every_document_is_listed_with_its_size_and_an_abbreviated_hash() {
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));
    let first = publication.measured.first().expect("documents");

    let listing = listing(&publication);

    assert!(
        listing.contains(&format!("{}…", &first.hash[..16])),
        "a review is not a 64-character comparison: {listing}"
    );
    assert!(
        !listing.contains(&first.hash[..]),
        "the whole digest belongs in the index and in --out, not in a table"
    );
    assert!(
        listing.trim_end().ends_with(&format!(
            "{} documents, {} bytes",
            publication.measured.len(),
            publication.measured.iter().map(|d| d.bytes).sum::<usize>()
        )),
        "{listing}"
    );
}

#[test]
fn the_index_is_listed_last_because_it_is_published_last() {
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));

    let addresses: Vec<String> = publication
        .documents()
        .map(|document| document.address.to_string())
        .collect();

    assert_eq!(
        addresses.last().map(String::as_str),
        Some("index"),
        "an index naming a set of hashes implies those documents are already there (§7)"
    );
    assert_eq!(addresses.len(), publication.snapshot.documents.len() + 1);
}
