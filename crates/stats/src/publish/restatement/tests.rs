//! The rule of §8, as a rule: what a document's revision is, and which
//! documents a run does not send.

use super::*;
use crate::bucket::Coverage;
use crate::publish::index::Publisher;
use crate::series::Data;
use crate::window::Window;

/// 2026-08-27T03:06:40Z, the clock the E2E suite freezes.
const NOW: i64 = 1_787_800_000;
const A_WEEK: i64 = 604_800;

fn snapshot(now: i64) -> Snapshot {
    Snapshot::compute(&Data::default(), Coverage::since(now - 86_400), "run", now)
}

/// The address every test below reasons about, and the first document of
/// a snapshot in a stable order.
fn first(snapshot: &Snapshot) -> &Document {
    snapshot.documents.first().expect("a document")
}

/// What a previous run would have recorded for `document`: a first
/// publication, which is the state most of these tests start from.
fn published(document: &Document, updated_at: i64) -> (String, Previous) {
    (
        document.address.to_string(),
        Previous::First {
            hash: document.hash.clone(),
            updated_at,
        },
    )
}

/// The same, at a revision above the first — which by §8 is a
/// restatement and says why.
fn restated_at(
    document: &Document,
    revision: u32,
    updated_at: i64,
    because: Because,
) -> (String, Previous) {
    (
        document.address.to_string(),
        Previous::restated(
            document.hash.clone(),
            revision,
            updated_at,
            Restatement {
                at: updated_at,
                because: because.as_str().to_string(),
            },
        )
        .expect("a revision above the first"),
    )
}

fn history(entries: impl IntoIterator<Item = (String, Previous)>) -> BTreeMap<String, Previous> {
    entries.into_iter().collect()
}

// ---- a first publication

#[test]
fn a_document_nobody_published_is_revision_one_and_restates_nothing() {
    let snapshot = snapshot(NOW);

    let restated = snapshot.restated(&history([]), Because::Backfill, Republish::No);

    let document = first(&restated.snapshot);
    assert_eq!(document.envelope.revision(), 1);
    assert_eq!(document.envelope.restated_at(), None);
    assert_eq!(document.envelope.restated_because(), None);
    assert_eq!(document.updated_at, NOW);
}

#[test]
fn a_first_publication_sends_every_document() {
    let snapshot = snapshot(NOW);

    let restated = snapshot.restated(&history([]), Because::Backfill, Republish::No);

    assert!(
        restated.not_sent.is_empty(),
        "nothing is on the relay yet: {:?}",
        restated.not_sent
    );
}

// ---- an unchanged payload (§8)

#[test]
fn a_payload_that_did_not_move_keeps_its_revision_and_its_clock() {
    // The whole point of hashing the payload rather than the content: a
    // new run is not a new revision.
    let snapshot = snapshot(NOW);
    let earlier = NOW - 604_800;
    let history = history([restated_at(first(&snapshot), 3, earlier, Because::Rebuild)]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    let document = first(&restated.snapshot);
    assert_eq!(document.envelope.revision(), 3);
    assert_eq!(
        document.updated_at, earlier,
        "the figures last moved a week ago, whatever this run's clock says"
    );
}

#[test]
fn a_payload_that_did_not_move_is_not_sent() {
    let snapshot = snapshot(NOW);
    let address = first(&snapshot).address.to_string();
    let history = history([published(first(&snapshot), NOW - 60)]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    assert!(
        restated.not_sent.contains(&address),
        "the relay already holds it: {:?}",
        restated.not_sent
    );
}

#[test]
fn an_unchanged_document_is_still_in_the_index() {
    // "Unchanged" is one of the things the index exists to say; leaving
    // it out would read as "no longer published".
    let snapshot = snapshot(NOW);
    let history = history([published(first(&snapshot), NOW - 60)]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    let index = restated.snapshot.index(&Publisher {
        name: "bestiario".to_string(),
        version: "0".to_string(),
    });
    let payload = index.envelope.payload().to_string();
    assert!(
        payload.contains(&first(&snapshot).address.to_string()),
        "an unchanged document fell out of the index: {payload}"
    );
    assert_eq!(
        restated.snapshot.documents.len(),
        snapshot.documents.len(),
        "the index is built from every document, sent or not"
    );
}

#[test]
fn an_unchanged_document_keeps_the_restatement_it_already_carried() {
    // Revision 2 stays revision 2, and goes on saying why it became one.
    let snapshot = snapshot(NOW);
    let history = history([restated_at(first(&snapshot), 2, NOW - 60, Because::Rebuild)]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    let document = first(&restated.snapshot);
    assert_eq!(document.envelope.revision(), 2);
    assert_eq!(document.envelope.restated_because(), Some("rebuild"));
}

// ---- a payload that moved (§8)

#[test]
fn a_payload_that_moved_takes_the_next_revision_and_says_why() {
    let snapshot = snapshot(NOW);
    let history = history([(
        first(&snapshot).address.to_string(),
        Previous::restated(
            "a hash from another archive",
            4,
            NOW - 604_800,
            Restatement {
                at: NOW - 604_800,
                because: Because::Rebuild.as_str().to_string(),
            },
        )
        .expect("a revision above the first"),
    )]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    let document = first(&restated.snapshot);
    assert_eq!(document.envelope.revision(), 5);
    assert_eq!(document.envelope.restated_because(), Some("backfill"));
    assert_eq!(
        document.envelope.restated_at(),
        Some(crate::publish::document::rfc3339(NOW).as_str())
    );
    assert_eq!(document.updated_at, NOW);
    assert!(restated.not_sent.is_empty(), "it moved, so it is sent");
}

#[test]
fn a_first_revision_cannot_be_recorded_as_a_restatement() {
    // The invariant the enum exists for: nothing can hand the publisher a
    // revision 1 that claims to restate something.
    assert_eq!(
        Previous::restated(
            "hash",
            1,
            NOW,
            Restatement {
                at: NOW,
                because: Because::Backfill.as_str().to_string(),
            },
        ),
        None
    );
}

#[test]
fn every_reason_of_the_spec_has_a_word_a_client_can_match_on() {
    // A closed set, because a client renders it: a fifth reason nobody
    // agreed on would reach a rendering with no case for it.
    assert_eq!(Because::Backfill.as_str(), "backfill");
    assert_eq!(Because::Rebuild.as_str(), "rebuild");
    assert_eq!(Because::Schema.as_str(), "schema");
    assert_eq!(Because::Correction.as_str(), "correction");
}

// ---- --republish (§9.3)

#[test]
fn republishing_sends_the_documents_an_ordinary_run_would_skip() {
    // The documents a pruned relay is missing are overwhelmingly the ones
    // whose figures have not moved in months. A run that honoured the
    // skip would send it nothing at all.
    let snapshot = snapshot(NOW);
    let history = history(
        snapshot
            .documents
            .iter()
            .map(|document| published(document, NOW - 604_800)),
    );

    let restated = snapshot.restated(&history, Because::Backfill, Republish::All);

    assert!(
        restated.not_sent.is_empty(),
        "--republish skips nothing: {:?}",
        restated.not_sent
    );
}

#[test]
fn republishing_an_unchanged_payload_is_not_a_restatement() {
    // §9.3: it is one publication run like any other. Bumping a revision
    // for a re-signature would tell every client the figure changed.
    let snapshot = snapshot(NOW);
    let earlier = NOW - 604_800;
    let history = history([restated_at(first(&snapshot), 3, earlier, Because::Schema)]);

    let restated = snapshot.restated(&history, Because::Correction, Republish::All);

    let document = first(&restated.snapshot);
    assert_eq!(document.envelope.revision(), 3);
    assert_eq!(
        document.envelope.restated_because(),
        Some("schema"),
        "the restatement it already carried, not this run's reason"
    );
    assert_eq!(document.updated_at, earlier);
}

// ---- the run around it

#[test]
fn restating_changes_no_figure_and_no_hash() {
    // The payload is the answer; a revision is a fact about publication.
    // If this ever diverges, the index's hash stops meaning "the figures
    // a client already has".
    let snapshot = snapshot(NOW);
    let history = history([restated_at(
        first(&snapshot),
        9,
        NOW - 60,
        Because::Correction,
    )]);

    let restated = snapshot.restated(&history, Because::Schema, Republish::No);

    for (before, after) in snapshot.documents.iter().zip(&restated.snapshot.documents) {
        assert_eq!(before.hash, after.hash, "{}", before.address);
        assert_eq!(
            before.envelope.payload(),
            after.envelope.payload(),
            "{}",
            before.address
        );
        assert_eq!(before.address, after.address);
    }
    assert_eq!(restated.snapshot.run, snapshot.run);
    assert_eq!(restated.snapshot.coverage, snapshot.coverage);
}

#[test]
fn a_history_naming_an_address_this_run_did_not_compute_is_ignored() {
    // A `d` that fell out of the snapshot — a partition outside coverage
    // after a narrower rebuild — is not a document to publish. It stays
    // on the relay under its own address until something supersedes it.
    let snapshot = snapshot(NOW);
    let history = history([(
        "series:orders:daily:1999-01".to_string(),
        Previous::First {
            hash: "whatever".to_string(),
            updated_at: 0,
        },
    )]);

    let restated = snapshot.restated(&history, Because::Backfill, Republish::No);

    assert_eq!(restated.snapshot.documents.len(), snapshot.documents.len());
    assert!(restated.not_sent.is_empty());
}

// ---- what the run leaves behind

#[test]
fn the_state_a_run_records_is_what_the_next_run_would_compare_against() {
    // Round-trip: publish once against no history, feed the result back,
    // and nothing has moved — which is what "the figures did not change"
    // has to mean two runs in a row.
    let snapshot = snapshot(NOW);

    let first_run = snapshot.restated(&history([]), Because::Backfill, Republish::No);
    let second_run = snapshot.restated(&first_run.state, Because::Backfill, Republish::No);

    assert_eq!(
        second_run.not_sent.len(),
        snapshot.documents.len(),
        "a second run over an unchanged archive sends nothing"
    );
    assert_eq!(second_run.state, first_run.state, "and records the same");
}

#[test]
fn a_recorded_restatement_survives_the_round_trip() {
    // The record is what carries `restated_because` forward to every
    // later index, long after the run that caused it.
    let snapshot = snapshot(NOW);
    let history = history([(
        first(&snapshot).address.to_string(),
        Previous::First {
            hash: "a hash from another archive".to_string(),
            updated_at: NOW - 604_800,
        },
    )]);

    let restated = snapshot.restated(&history, Because::Rebuild, Republish::No);
    let recorded = restated
        .state
        .get(&first(&snapshot).address.to_string())
        .expect("recorded");

    assert_eq!(recorded.revision(), 2);
    assert_eq!(recorded.updated_at(), NOW);
    assert_eq!(
        recorded.restatement().map(|r| r.because.as_str()),
        Some("rebuild")
    );

    // And a run after it, with nothing further changed, repeats it.
    let again = snapshot.restated(&restated.state, Because::Backfill, Republish::No);
    assert_eq!(
        first(&again.snapshot).envelope.restated_because(),
        Some("rebuild")
    );
}

// ---- why the figures moved (§8)

use crate::publish::document::SCHEMA_VERSION;

/// A reading of the archive: the schema published under, how far back it
/// reached, and how much it held.
fn read(schema_version: u32, covered_from: Option<i64>, events: u64) -> Read {
    Read {
        schema_version,
        covered_from,
        events,
    }
}

#[test]
fn a_schema_bump_is_why_everything_moved_at_once() {
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION + 1, Some(NOW - 86_400), 100),
            read(SCHEMA_VERSION, Some(NOW - 86_400), 100),
        ),
        Because::Schema
    );
}

#[test]
fn the_schema_is_checked_before_anything_the_archive_says() {
    // A schema bump moves every payload at once; a backfill in the same
    // run is the smaller half of the story.
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION - 1, Some(NOW), 100),
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 200),
        ),
        Because::Schema
    );
}

#[test]
fn an_archive_that_reaches_further_back_was_backfilled() {
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, Some(NOW - 86_400), 100),
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 100),
        ),
        Because::Backfill
    );
}

#[test]
fn an_archive_that_holds_more_events_was_backfilled_even_at_the_same_depth() {
    // A walk that finds events *inside* a range already covered moves no
    // floor and moves plenty of figures. Reading that as a rebuild would
    // tell every client the projections changed when the events did.
    let floor = Some(NOW - A_WEEK);
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, floor, 100),
            read(SCHEMA_VERSION, floor, 101),
        ),
        Because::Backfill
    );
}

#[test]
fn an_archive_that_held_nothing_and_now_holds_something_was_backfilled() {
    // Reaching back from nowhere is the extreme case of reaching back,
    // not a case of its own.
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, None, 0),
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 12),
        ),
        Because::Backfill
    );
}

#[test]
fn figures_that_moved_over_the_same_events_were_rebuilt() {
    let floor = Some(NOW - A_WEEK);
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, floor, 100),
            read(SCHEMA_VERSION, floor, 100),
        ),
        Because::Rebuild
    );
}

#[test]
fn an_archive_that_lost_events_did_not_reach_further_back() {
    // Nothing was backfilled into an archive that now holds less. A
    // `rebuild --from-raw` that drops a version is the case, and the
    // figures moved because the projections did.
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 100),
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 90),
        ),
        Because::Rebuild
    );
    assert_eq!(
        Because::inferred(
            read(SCHEMA_VERSION, Some(NOW - A_WEEK), 100),
            read(SCHEMA_VERSION, None, 0),
        ),
        Because::Rebuild
    );
    assert_eq!(
        Because::inferred(read(SCHEMA_VERSION, None, 0), read(SCHEMA_VERSION, None, 0)),
        Because::Rebuild
    );
}

// ---- --republish over a range (§9.3)

/// The partitions a snapshot computed, with the span each one covers.
fn partitions(snapshot: &Snapshot) -> Vec<(String, Window)> {
    snapshot
        .documents
        .iter()
        .filter_map(|document| {
            document
                .period
                .map(|period| (document.address.to_string(), period))
        })
        .collect()
}

/// A range nothing bestiario ever published falls inside.
const NOTHING: Window = Window {
    from: 0,
    until: 86_400,
};

#[test]
fn a_range_republishes_the_partitions_it_touches_and_no_others() {
    let snapshot = snapshot(NOW);
    let history = history(
        snapshot
            .documents
            .iter()
            .map(|document| published(document, NOW - A_WEEK)),
    );
    let (touched, period) = partitions(&snapshot).first().cloned().expect("a partition");

    let restated = snapshot.restated(&history, Because::Backfill, Republish::Range(period));

    assert!(
        !restated.not_sent.contains(&touched),
        "the range covers {touched} exactly and it was withheld"
    );
    assert!(
        restated.not_sent.contains("orders:24h"),
        "a window document covers no fixed span, so no range names it: {:?}",
        restated.not_sent
    );
}

#[test]
fn a_range_that_touches_nothing_sends_nothing_it_did_not_already_have_to() {
    let snapshot = snapshot(NOW);
    let history = history(
        snapshot
            .documents
            .iter()
            .map(|document| published(document, NOW - A_WEEK)),
    );

    let restated = snapshot.restated(&history, Because::Backfill, Republish::Range(NOTHING));

    assert_eq!(
        restated.not_sent.len(),
        snapshot.documents.len(),
        "nothing in 1970 was published, so nothing is recovered"
    );
}

#[test]
fn a_range_still_sends_a_document_that_changed_outside_it() {
    // The index names every document with the hash of the payload that
    // belongs to it (§7). Withholding a changed document outside the
    // range would publish an index pointing at something no relay holds.
    let snapshot = snapshot(NOW);
    let outside = snapshot
        .documents
        .iter()
        .find(|document| document.period.is_none())
        .expect("a window document");
    let history = history(snapshot.documents.iter().map(|document| {
        if document.address == outside.address {
            (
                document.address.to_string(),
                Previous::First {
                    hash: "a hash from another archive".to_string(),
                    updated_at: NOW - A_WEEK,
                },
            )
        } else {
            published(document, NOW - A_WEEK)
        }
    }));

    let restated = snapshot.restated(&history, Because::Backfill, Republish::Range(NOTHING));

    assert!(
        !restated.not_sent.contains(&outside.address.to_string()),
        "{} moved and was withheld anyway",
        outside.address
    );
}
