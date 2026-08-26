//! Reading the `y` tag, against the captured corpus and against hand-built
//! shapes the corpus does not contain.

use nostr_sdk::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};

use super::*;
use crate::ingest::parse::fixtures::load;

/// An order carrying the given `y` values, or none at all.
fn order_with_y(values: &[&str]) -> Event {
    let mut builder = EventBuilder::new(Kind::from_u16(38383), "");
    if !values.is_empty() {
        let tag = Tag::parse(
            std::iter::once("y")
                .chain(values.iter().copied())
                .collect::<Vec<_>>(),
        )
        .expect("well-formed tag");
        builder = builder.tag(tag);
    }

    builder.finalize(&Keys::generate()).expect("signing")
}

#[test]
fn a_named_instance_yields_its_name() {
    let name = instance_name(&load(38383, "canceled"));

    assert_eq!(name.as_deref(), Some("Mostro Brasil"));
}

#[test]
fn a_nameless_instance_yields_none_on_every_kind_that_carries_the_tag() {
    // `y = ["mostro"]` is what a third of the network publishes, so this is
    // the common case and not an error.
    for (kind, fixture) in [
        (38386, "without_instance_name"),
        (38385, "without_instance_name"),
    ] {
        assert_eq!(instance_name(&load(kind, fixture)), None, "{kind}");
        assert!(is_mostro(&load(kind, fixture)), "{kind}");
    }
}

#[test]
fn the_platform_of_a_mostro_event_is_mostro() {
    let event = load(38383, "pending_range");

    assert_eq!(platform(&event).as_deref(), Some(MOSTRO));
    assert!(is_mostro(&event));
}

#[test]
fn an_order_from_another_platform_is_not_mostro() {
    // The relays carry NIP-69 orders from telegram bots, hodlhodl and others.
    // Indexing them would fold four other platforms' activity into the Mostro
    // figures (SPEC 2.1).
    for fixture in [
        "other_platform_telegram",
        "other_platform_hodlhodl",
        "other_platform_bitway",
    ] {
        let event = load(38383, fixture);

        assert!(!is_mostro(&event), "{fixture}: {:?}", platform(&event));
        assert!(platform(&event).is_some(), "{fixture} does carry a y tag");
    }
}

#[test]
fn the_kinds_without_a_y_tag_have_no_platform_and_no_name() {
    // Rates and relay lists publish no `y`, which is why the platform filter
    // is scoped to the kinds that carry one: applied to these it would
    // discard every rate and every relay list.
    for kind in [30078, 10002] {
        let event = load(kind, "typical");

        assert_eq!(platform(&event), None, "{kind}");
        assert_eq!(instance_name(&event), None, "{kind}");
        assert!(!is_mostro(&event), "{kind}");
    }
}

#[test]
fn an_event_with_no_y_tag_at_all_yields_none() {
    let event = order_with_y(&[]);

    assert_eq!(platform(&event), None);
    assert_eq!(instance_name(&event), None);
}

#[test]
fn a_blank_name_is_no_name() {
    // Storing it would win the most-recent-name-wins rule of SPEC 3 and blank
    // out a name the instance published a minute earlier.
    for blank in ["", "   "] {
        let event = order_with_y(&[MOSTRO, blank]);

        assert_eq!(instance_name(&event), None, "{blank:?}");
    }
}

#[test]
fn a_name_is_trimmed_but_otherwise_kept_verbatim() {
    // Instance names carry flags, emoji and spaces; only the padding goes.
    let event = order_with_y(&[MOSTRO, "  MostroColomBia🇨🇴  "]);

    assert_eq!(instance_name(&event).as_deref(), Some("MostroColomBia🇨🇴"));
}

#[test]
fn a_third_value_is_ignored_rather_than_read_as_a_name() {
    let event = order_with_y(&[MOSTRO, "Mostro", "something else"]);

    assert_eq!(instance_name(&event).as_deref(), Some("Mostro"));
}

#[test]
fn a_repeated_y_tag_names_no_platform() {
    // Two `y` tags name two platforms and neither can be preferred, so the
    // event falls out of the filter rather than being indexed under a guess.
    let event = EventBuilder::new(Kind::from_u16(38383), "")
        .tag(Tag::parse(["y", MOSTRO, "Mostro Brasil"]).expect("well-formed tag"))
        .tag(Tag::parse(["y", "hodlhodl"]).expect("well-formed tag"))
        .finalize(&Keys::generate())
        .expect("signing");

    assert_eq!(platform(&event), None);
    assert_eq!(instance_name(&event), None);
    assert!(!is_mostro(&event));
}
