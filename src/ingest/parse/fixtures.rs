//! Loading the captured events under `tests/fixtures` from unit tests.
//!
//! The parsers are tested against the same corpus the integration tests
//! guard (`tests/fixtures/README.md`): real signed events, so that a parser
//! is written against what the network publishes rather than against what a
//! hand-written literal says it publishes.

use nostr_sdk::prelude::{Event, EventBuilder, FinalizeEvent as _, Keys, SecretKey, Tag};

/// `event` as a local relay will keep it: without its NIP-40 `expiration`,
/// signed by a key derived from the instance's real pubkey.
///
/// Every captured event carries an `expiration` tag, and a relay — the
/// `nostr-sdk` local one included — refuses an expired event. Orders expire
/// within a day of publication, so a test that seeds a relay with the
/// events as captured starts losing them a day after the capture, one by
/// one, and fails without anything having changed. The event is rebuilt
/// without the tag and signed with a key derived from the real pubkey, so
/// the same instance is the same pubkey on every run: every other tag, the
/// timestamp and the identifier are the instance's own, and the signature
/// verifies. Parser tests keep using [`load`] as captured; only tests that
/// put events *on a relay* need this.
///
/// A kind 38385 is keyed on its own pubkey (`d` = pubkey hex, SPEC §2.4),
/// and the parser checks that; the `d` follows the key.
pub(crate) fn for_relay(event: &Event) -> Event {
    let keys = Keys::new(
        SecretKey::from_slice(&event.pubkey.to_bytes()).expect("32 random bytes are a scalar"),
    );
    let tags = event
        .tags
        .iter()
        .filter(|tag| tag.kind() != "expiration")
        .map(|tag| {
            if event.kind.as_u16() == 38385 && tag.kind() == "d" {
                Tag::identifier(keys.public_key().to_hex())
            } else {
                tag.clone()
            }
        });

    EventBuilder::new(event.kind, event.content.clone())
        .tags(tags)
        .custom_created_at(event.created_at)
        .finalize(&keys)
        .expect("sign")
}

/// The fixture `tests/fixtures/{kind}/{name}.json`.
///
/// Panics rather than returning a `Result`: a missing or malformed fixture is
/// a broken test, not a test failure worth a message of its own.
/// Every fixture of `kind`, in file-name order.
pub(crate) fn corpus(kind: u16) -> Vec<Event> {
    let dir = format!("{}/tests/fixtures/{kind}", env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {dir}: {e}"))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            path.file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names.iter().map(|name| load(kind, name)).collect()
}

pub(crate) fn load(kind: u16, name: &str) -> Event {
    let path = format!(
        "{}/tests/fixtures/{kind}/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    Event::from_json(&json).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}
