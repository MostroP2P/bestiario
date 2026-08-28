//! The decisions `publish` makes on its own: the run's identity, the
//! ceiling it refuses against, and what a review prints.

use nostr_sdk::prelude::{Filter, Keys, Kind, MockRelay};

use super::*;
use crate::stats::series::Data;
use bestiario_stats::publish::restatement::Previous;

/// 2026-08-27T03:06:40Z, the clock the E2E suite freezes.
const NOW: i64 = 1_787_800_000;

fn publication(coverage: Coverage, ceiling: Ceiling) -> Publication {
    published_after(coverage, ceiling, &BTreeMap::new())
}

/// A publication computed against `history`, the way `compute` builds one
/// — without the database it would otherwise read the history from.
fn published_after(
    coverage: Coverage,
    ceiling: Ceiling,
    history: &BTreeMap<String, Previous>,
) -> Publication {
    let Restated {
        snapshot,
        mut not_sent,
        mut state,
    } = Snapshot::compute(&Data::default(), coverage, &snapshot_id(NOW), NOW).restated(
        history,
        Because::Backfill,
        Republish::No,
    );
    let index = snapshot.index(&publisher());
    let address = index.address.to_string();
    if history
        .get(&address)
        .is_some_and(|previous| previous.hash() == index.hash)
    {
        not_sent.insert(address.clone());
    }
    state.insert(address, restatement::recorded(&index));
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
        not_sent,
        state,
        events: 0,
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

// ---- signing and relay publication (§7, §12)

/// A throwaway key, used nowhere but here.
const NSEC: &str = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";

fn keys() -> Keys {
    crate::nostr::signer::parse(NSEC, "the test key").expect("a key")
}

#[tokio::test]
async fn every_document_and_the_index_reach_the_relay() {
    let relay = MockRelay::run().await.expect("start the local relay");
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));
    let url = relay.url().await.to_string();

    let report = send(&publication, &keys(), std::slice::from_ref(&url))
        .await
        .expect("publish");

    let client = RelayClient::connect(&[url]).await.expect("connect");
    let stored = client
        .fetch_window(
            &client.relays()[0],
            Filter::new().kind(Kind::Custom(crate::stats::publish::document::KIND)),
        )
        .await
        .expect("read them back");

    assert_eq!(
        stored.len(),
        publication.measured.len(),
        "the relay holds a different number of documents than were sent"
    );
    let addresses: std::collections::BTreeSet<_> = stored
        .iter()
        .filter_map(|event| event.tags.identifier())
        .collect();
    for document in publication.documents() {
        assert!(
            addresses.contains(&document.address.to_string()),
            "{} never reached the relay",
            document.address
        );
    }
    assert!(
        report.contains(&publication.snapshot.run.snapshot_id),
        "the report has to name the run it published: {report}"
    );
}

#[tokio::test]
async fn a_document_no_relay_took_stops_the_index_that_would_name_it() {
    // §7: an index on a relay is a promise that the documents it names are
    // already there. Sending it anyway would leave readers fetching
    // something that is not there until the next run.
    let relay = MockRelay::run().await.expect("start the local relay");
    let url = relay.url().await.to_string();
    let publication = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));

    let client = RelayClient::connect(&[url]).await.expect("connect");
    relay.shutdown();

    let refusal = send_to(&publication, &keys(), &client)
        .await
        .expect_err("no relay is left to take anything");

    let message = refusal.to_string();
    assert!(
        message.contains("the index naming them was not published"),
        "the operator has to be told the index was withheld: {message}"
    );
}

// ---- what a second run over an unchanged archive sends (§8)

/// The history a run against `publication` would leave behind.
fn history_of(publication: &Publication) -> BTreeMap<String, Previous> {
    publication.state.clone()
}

#[tokio::test]
async fn a_run_over_an_unchanged_archive_sends_nothing_at_all() {
    // Not an optimisation: a relay does not need a second copy of an
    // answer that did not change, and every re-signature is a new event
    // every client has to decide it already had.
    let relay = MockRelay::run().await.expect("start the local relay");
    let url = relay.url().await.to_string();
    let first = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));
    send(&first, &keys(), std::slice::from_ref(&url))
        .await
        .expect("publish");

    let again = published_after(
        Coverage::since(NOW - 86_400),
        Ceiling::configured(65_536),
        &history_of(&first),
    );
    let report = send(&again, &keys(), std::slice::from_ref(&url))
        .await
        .expect("publish again");

    assert!(
        report.contains("nothing changed"),
        "the run has to say it sent nothing: {report}"
    );
    let client = RelayClient::connect(&[url]).await.expect("connect");
    let stored = client
        .fetch_window(
            &client.relays()[0],
            Filter::new().kind(Kind::Custom(crate::stats::publish::document::KIND)),
        )
        .await
        .expect("read them back");
    assert_eq!(
        stored.len(),
        first.measured.len(),
        "the second run added events for figures that did not move"
    );
}

#[test]
fn an_unchanged_run_still_lists_every_document_and_records_every_one() {
    // "Unchanged" is one of the things the index exists to say, and a
    // document dropped from the record would restart at revision 1.
    let first = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));

    let again = published_after(
        Coverage::since(NOW - 86_400),
        Ceiling::configured(65_536),
        &history_of(&first),
    );

    assert_eq!(again.measured.len(), first.measured.len());
    assert_eq!(again.state, first.state, "and records the same revisions");
    assert_eq!(
        again.not_sent.len(),
        again.measured.len(),
        "every document, the index included, is already published"
    );
}

#[test]
fn the_index_is_withheld_only_when_nothing_it_names_moved() {
    // The index's payload carries every document's hash, so an index that
    // did not move implies nothing it names did. The converse is what
    // §7 rests on: an index is never withheld while something it names
    // is being sent.
    let first = publication(Coverage::since(NOW - 86_400), Ceiling::configured(65_536));
    let mut history = history_of(&first);
    history.insert(
        "orders:24h".to_string(),
        Previous::First {
            hash: "a hash from another archive".to_string(),
            updated_at: NOW - 604_800,
        },
    );

    let again = published_after(
        Coverage::since(NOW - 86_400),
        Ceiling::configured(65_536),
        &history,
    );

    assert!(
        !again.not_sent.contains("orders:24h"),
        "the document moved and was withheld"
    );
    assert!(
        !again.not_sent.contains("index"),
        "the index names a hash nobody published yet and was withheld anyway"
    );
}
