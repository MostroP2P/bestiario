//! What a key may be spelled as, and what a signed document carries.

use super::*;
use crate::config::SecretRef;
use crate::stats::bucket::Coverage;
use crate::stats::publish::snapshot::Snapshot;
use crate::stats::series::Data;

/// A throwaway key, used nowhere but here.
const NSEC: &str = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";
const NSEC_HEX: &str = "67dea2ed018072d675f5415ecfaed7d2597555e202d85b3d65ea4e58d2d92ffa";

/// 2026-08-27T03:06:40Z, the clock the E2E suite freezes.
const NOW: i64 = 1_787_800_000;

/// The first value of the tag named `name`, which is how every tag of §11
/// is read: 0.45 has no typed accessor for a tag it does not know.
fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.kind() == name)
        .and_then(Tag::content)
}

fn snapshot() -> Snapshot {
    Snapshot::compute(
        &Data::default(),
        Coverage::since(NOW - 86_400),
        "20260827T030640Z",
        NOW,
    )
}

// ---- the key (§12)

#[test]
fn the_two_spellings_of_one_key_are_the_same_key() {
    // An operator pastes whichever their key manager printed; refusing
    // one of them would be a rule with nothing behind it.
    let bech32 = parse(NSEC, "[publish].nsec").expect("an nsec");
    let hex = parse(NSEC_HEX, "[publish].nsec").expect("the same key as hex");

    assert_eq!(bech32.public_key(), hex.public_key());
}

#[test]
fn surrounding_whitespace_is_not_part_of_a_key() {
    // A key file written by `echo` ends in a newline, and a newline is
    // not a checksum failure worth an error message.
    let keys = parse(&format!("  {NSEC}\n"), "a key file").expect("a key with a newline");

    assert_eq!(
        keys.public_key(),
        parse(NSEC, "x").expect("a key").public_key()
    );
}

#[test]
fn a_malformed_key_names_the_setting_it_came_from() {
    let error = parse("nsec1nonsense", "[publish].nsec").expect_err("not a key");

    assert!(
        error.to_string().contains("[publish].nsec"),
        "an operator with two places to look needs to be told which: {error}"
    );
}

#[test]
fn a_public_key_is_not_a_signing_key() {
    // The one paste that would otherwise half-work: an npub is bech32 and
    // is the right length, and signing with it is not possible.
    let npub = "npub1sn0wdenkukak0d9dfczzeacvhkrgz92ak56egt7vdgzn8pv2wfqqhrjdv9";

    parse(npub, "[publish].nsec").expect_err("a public key cannot sign");
}

// ---- where the key comes from (§12)

/// `[publish].nsec = "env:NAME"`, parsed the way the configuration does.
fn reference(name: &str) -> SecretRef {
    #[derive(serde::Deserialize)]
    struct Holder {
        nsec: SecretRef,
    }
    toml::from_str::<Holder>(&format!("nsec = \"env:{name}\""))
        .expect("a reference")
        .nsec
}

#[test]
fn a_variable_nobody_exported_is_named_in_the_refusal() {
    // The operator picked the name; it is the only thing that tells them
    // which variable to export.
    let error = resolve(&reference("BESTIARIO_NOTHING_EXPORTED_THIS")).expect_err("not exported");

    assert!(
        error
            .to_string()
            .contains("BESTIARIO_NOTHING_EXPORTED_THIS"),
        "the refusal has to name the variable: {error}"
    );
}

// ---- the signed event (§2, §11)

#[test]
fn a_signed_document_is_the_addressable_kind_and_verifies() {
    let keys = parse(NSEC, "x").expect("a key");
    let snapshot = snapshot();
    let document = snapshot.documents.first().expect("a document");

    let event = sign(document, &snapshot.run, &keys);

    assert_eq!(event.kind, Kind::Custom(KIND));
    assert_eq!(event.pubkey, keys.public_key());
    event.verify().expect("a signature the relay will check");
}

#[test]
fn the_events_content_is_the_documents_envelope_and_its_d_tag_the_address() {
    // The two things a client reads: what to look the event up by, and
    // what it says. A mismatch between them is the one publication error
    // no reader could detect.
    let keys = parse(NSEC, "x").expect("a key");
    let snapshot = snapshot();
    let document = snapshot.documents.first().expect("a document");

    let event = sign(document, &snapshot.run, &keys);

    assert_eq!(event.content, document.content());
    assert_eq!(
        event.tags.identifier().as_deref(),
        Some(document.address.to_string().as_str())
    );
}

#[test]
fn every_document_of_a_run_carries_the_runs_snapshot_id_and_its_clock() {
    // §7: the `s` tag is how a client asks a relay for a whole run in one
    // filter, and `created_at` is the run's rather than the signature's,
    // so a run that takes a minute to sign is still one run.
    let keys = parse(NSEC, "x").expect("a key");
    let snapshot = snapshot();

    for document in &snapshot.documents {
        let event = sign(document, &snapshot.run, &keys);

        assert_eq!(
            tag(&event, "s"),
            Some(snapshot.run.snapshot_id.as_str()),
            "{} carries another run's identity",
            document.address
        );
        assert_eq!(event.created_at.as_secs(), NOW as u64);
    }
}

#[test]
fn a_document_is_discoverable_by_topic_and_states_its_schema() {
    let keys = parse(NSEC, "x").expect("a key");
    let snapshot = snapshot();
    let document = snapshot.documents.first().expect("a document");

    let event = sign(document, &snapshot.run, &keys);

    assert_eq!(tag(&event, "t"), Some(document::TOPIC));
    assert_eq!(
        tag(&event, "schema_version"),
        Some(document::SCHEMA_VERSION.to_string().as_str())
    );
}
