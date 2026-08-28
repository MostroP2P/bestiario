//! The tag set of `docs/NOSTR-PUBLICATION.md` §11 and the envelope of §6,
//! as a table.

use super::*;
use crate::publish::address::{
    Address, Bucket, Month, Partition, Report, Resolution, Scope, Window, Year,
};

fn window_address() -> Address {
    Address::Window {
        report: Report::Summary,
        window: Window::Days30,
        scope: None,
    }
}

/// A year, and the bucket covering one month: the bounds are the address
/// module's own test, not this one's.
fn year_of(year: i32) -> Year {
    Year::new(year).expect("a four-digit year")
}

fn month_of(year: i32, month: u32) -> Bucket {
    Bucket::Month {
        year: year_of(year),
        month: Month::new(month).expect("a month of the year"),
    }
}

fn series_address() -> Address {
    Address::Series {
        report: Report::Orders,
        partition: Partition::new(Resolution::Daily, month_of(2026, 1)).expect("a month of days"),
        scope: Some(Scope::Network("mainnet".into())),
    }
}

fn run() -> Run {
    Run {
        snapshot_id: "01J8ZTEST".into(),
        generated_at: 1_787_800_000,
    }
}

fn find<'a>(tags: &'a [Tag], name: &str) -> Vec<&'a str> {
    tags.iter()
        .filter(|tag| tag.name == name)
        .map(Tag::value)
        .collect()
}

#[test]
fn the_kind_is_the_one_the_spec_reserves() {
    assert_eq!(KIND, 30666);
}

#[test]
fn every_document_carries_the_indexed_tags_and_a_human_readable_alt() {
    // Arrange / Act
    let tags = tags(&window_address(), &run(), Some(1));

    // Assert: the three relay-indexed tags, and NIP-31's `alt`.
    assert_eq!(find(&tags, "d"), ["summary:30d"]);
    assert_eq!(find(&tags, "s"), ["01J8ZTEST"]);
    assert_eq!(find(&tags, "t"), ["bestiario"]);
    let alt = find(&tags, "alt");
    assert_eq!(alt.len(), 1);
    assert!(alt[0].contains("summary"), "{}", alt[0]);
    assert!(alt[0].contains("30 days"), "{}", alt[0]);
    assert_eq!(find(&tags, "revision"), ["1"]);
    assert_eq!(find(&tags, "schema_version"), [SCHEMA_VERSION.to_string()]);
}

#[test]
fn a_series_partition_also_names_its_resolution_and_period() {
    // The period is the address's own: January 2026, read off the bucket
    // rather than handed over beside it, so no series can lack one.
    let tags = tags(&series_address(), &run(), Some(3));

    assert_eq!(find(&tags, "resolution"), ["daily"]);
    let period_tag = tags
        .iter()
        .find(|tag| tag.name == "period")
        .expect("a period tag");
    assert_eq!(
        period_tag.values,
        vec!["2026-01-01T00:00:00+00:00", "2026-02-01T00:00:00+00:00"]
    );
    assert_eq!(find(&tags, "revision"), ["3"]);
}

#[test]
fn a_window_document_names_no_resolution_and_no_period() {
    let tags = tags(&window_address(), &run(), Some(1));

    assert!(find(&tags, "resolution").is_empty());
    assert!(find(&tags, "period").is_empty());
}

#[test]
fn the_alt_of_a_series_partition_reads_as_one_sentence() {
    let tags = tags(&series_address(), &run(), Some(1));
    let alt = find(&tags, "alt")[0];

    assert!(alt.contains("orders"), "{alt}");
    assert!(alt.contains("daily"), "{alt}");
    assert!(alt.contains("2026-01"), "{alt}");
    assert!(alt.contains("mainnet"), "{alt}");
}

#[test]
fn the_index_is_described_as_the_index() {
    let tags = tags(&Address::Index { year: None }, &run(), None);
    let alt = find(&tags, "alt")[0];

    assert!(alt.to_lowercase().contains("index"), "{alt}");
    assert_eq!(find(&tags, "d"), ["index"]);
}

#[test]
fn every_report_and_window_has_words_of_its_own_in_the_alt() {
    // Each variant renders to a phrase that names it; a table rather than
    // a spot check, since a new variant that fell through to a neighbour's
    // wording would describe one document as another.
    let alt_of = |address: &Address| find(&tags(address, &run(), Some(1)), "alt")[0].to_string();

    let mut seen = std::collections::BTreeSet::new();
    for report in Report::ALL {
        let alt = alt_of(&Address::Window {
            report,
            window: Window::Days30,
            scope: None,
        });
        assert!(
            seen.insert(alt.clone()),
            "{report:?} reads like another: {alt}"
        );
    }

    let mut seen = std::collections::BTreeSet::new();
    for window in Window::ALL {
        let alt = alt_of(&Address::Window {
            report: Report::Orders,
            window,
            scope: None,
        });
        assert!(
            seen.insert(alt.clone()),
            "{window:?} reads like another: {alt}"
        );
    }
    assert!(
        alt_of(&Address::Window {
            report: Report::Orders,
            window: Window::All,
            scope: None,
        })
        .contains("whole archive")
    );
}

#[test]
fn a_scope_and_a_year_are_spelled_out_in_the_alt() {
    let alt_of = |address: &Address| find(&tags(address, &run(), Some(1)), "alt")[0].to_string();
    let pubkey = "6320ee5e2ce0e1e0ae5d2a3e0b8f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6fd425";

    let instance = alt_of(&Address::Window {
        report: Report::Volume,
        window: Window::Days7,
        scope: Some(Scope::Instance(pubkey.into())),
    });
    assert!(instance.contains("for instance"), "{instance}");
    assert!(instance.contains(pubkey), "{instance}");

    let yearly = alt_of(&Address::Series {
        report: Report::Volume,
        partition: Partition::new(Resolution::Monthly, Bucket::Year(year_of(2026)))
            .expect("a year"),
        scope: None,
    });
    assert!(yearly.contains("year 2026"), "{yearly}");

    let sharded = alt_of(&Address::Index {
        year: Some(year_of(2026)),
    });
    assert!(sharded.contains("2026"), "{sharded}");
    assert!(sharded.to_lowercase().contains("index"), "{sharded}");
}

#[test]
fn only_single_letter_tags_are_the_relay_indexed_ones() {
    // §11: `d`, `s`, `t` are for the relay; everything else is for a
    // client that already holds the event.
    let tags = tags(&series_address(), &run(), Some(1));

    for tag in &tags {
        let indexed = tag.name.len() == 1;
        assert_eq!(
            indexed,
            ["d", "s", "t"].contains(&tag.name.as_str()),
            "{}",
            tag.name
        );
    }
}

#[test]
fn the_envelope_wraps_a_payload_with_the_run_and_the_revision() {
    let envelope = Envelope::first(&run(), serde_json::json!({"range": {}, "metrics": []}));
    let json = serde_json::to_value(&envelope).expect("serialises");

    assert_eq!(json["schema_version"], SCHEMA_VERSION);
    assert_eq!(json["snapshot_id"], "01J8ZTEST");
    assert_eq!(json["generated_at"], "2026-08-27T03:06:40+00:00");
    assert_eq!(json["revision"], 1);
    assert_eq!(json["payload"]["metrics"], serde_json::json!([]));
    // Field order is part of the format: run first, answer last.
    let text = serde_json::to_string(&envelope).expect("serialises");
    let order: Vec<usize> = [
        "schema_version",
        "snapshot_id",
        "generated_at",
        "revision",
        "payload",
    ]
    .iter()
    .map(|field| text.find(&format!("\"{field}\"")).expect(field))
    .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

#[test]
fn a_restated_envelope_says_when_and_why() {
    let envelope = Envelope::restated(
        &run(),
        2,
        Restatement {
            at: 1_787_800_000,
            because: "the rate book gained a snapshot that priced three orders".into(),
        },
        serde_json::json!({}),
    )
    .expect("revision 2 is a restatement");
    let json = serde_json::to_value(&envelope).expect("serialises");

    assert_eq!(json["revision"], 2);
    assert_eq!(json["restated_at"], "2026-08-27T03:06:40+00:00");
    assert!(
        json["restated_because"]
            .as_str()
            .unwrap()
            .contains("rate book")
    );

    // And a first revision carries neither field at all: an absent
    // restatement is not a restatement with empty reasons.
    let plain =
        serde_json::to_value(Envelope::first(&run(), serde_json::json!({}))).expect("serialises");
    assert_eq!(plain["revision"], 1);
    assert!(plain.get("restated_at").is_none());
    assert!(plain.get("restated_because").is_none());
}

#[test]
fn a_revision_above_the_first_cannot_lack_its_restatement_and_the_first_cannot_have_one() {
    // §8: there is no way to build a revision 2 without saying why, and no
    // way to call revision 1 a restatement — the two states are the API.
    let restatement = Restatement {
        at: 1_787_800_000,
        because: "backfill".into(),
    };

    assert!(Envelope::restated(&run(), 1, restatement.clone(), serde_json::json!({})).is_none());
    assert!(Envelope::restated(&run(), 0, restatement, serde_json::json!({})).is_none());
    assert_eq!(Envelope::first(&run(), serde_json::json!({})).revision(), 1);
}

#[test]
fn an_envelope_gives_back_what_it_was_built_from() {
    // The fields are private so the two states stay coupled; the readers
    // are what the snapshot and the index build their records from.
    let payload = serde_json::json!({"range": {}, "metrics": [1, 2, 3]});
    let envelope = Envelope::first(&run(), payload.clone());

    assert_eq!(envelope.snapshot_id(), "01J8ZTEST");
    assert_eq!(envelope.generated_at(), "2026-08-27T03:06:40+00:00");
    assert_eq!(envelope.revision(), 1);
    assert_eq!(envelope.payload(), &payload);
}
