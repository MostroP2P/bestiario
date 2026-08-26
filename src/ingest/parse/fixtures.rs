//! Loading the captured events under `tests/fixtures` from unit tests.
//!
//! The parsers are tested against the same corpus the integration tests
//! guard (`tests/fixtures/README.md`): real signed events, so that a parser
//! is written against what the network publishes rather than against what a
//! hand-written literal says it publishes.

use nostr_sdk::prelude::Event;

/// The fixture `tests/fixtures/{kind}/{name}.json`.
///
/// Panics rather than returning a `Result`: a missing or malformed fixture is
/// a broken test, not a test failure worth a message of its own.
pub(crate) fn load(kind: u16, name: &str) -> Event {
    let path = format!(
        "{}/tests/fixtures/{kind}/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    Event::from_json(&json).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}
