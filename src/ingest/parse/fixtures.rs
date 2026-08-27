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
