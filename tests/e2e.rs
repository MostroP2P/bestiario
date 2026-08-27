//! End to end, `docs/SPEC.md` §12: a local relay holding the captured
//! fixtures, the real binary run against it — `backfill`, then every report
//! command with `--json` — and the summary compared against a committed
//! expected file.
//!
//! The other reports are checked for shape rather than content: `instances`
//! carries `silent_for`, which is a function of the clock, and a file that
//! changed every run would be a file nobody reads. The summary has no such
//! figure, so it is pinned exactly.
//!
//! # Updating the expected file
//!
//! `E2E_UPDATE=1 cargo test --test e2e` rewrites `tests/expected/summary.json`
//! from the current output. Read the diff before committing it: the point of
//! the file is that a change here is a change someone decided on.
//!
//! # Capturing the README examples
//!
//! `E2E_DUMP_DIR=<dir> cargo test --test e2e` also writes the *table*
//! rendering of every report to `<dir>/<command>.txt`, so the worked
//! examples in `README.md` are real output of this corpus rather than
//! something typed from memory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nostr_sdk::prelude::{
    Event, EventBuilder, FinalizeEvent as _, Keys, MockRelay, PublicKey, SecretKey,
};
use tempfile::TempDir;

/// The kinds the backfill walks, in the fixture directories of the same
/// name. Rates and relay lists are phase 3 and 4.
const KINDS: [u16; 4] = [38383, 8383, 38386, 38385];

/// A window around the capture date of the fixtures (2026-08-26).
const FROM: &str = "1787500000";
const UNTIL: &str = "1787800000";

/// The instance named "Mostro" — `82fa8cb9…` on the network; see
/// [`test_keys`] for what it becomes on the local relay.
const MOSTRO: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The keys an instance signs with on the local relay: derived from its
/// real pubkey, so the same instance is the same pubkey on every run.
///
/// # Why the corpus is re-signed for the relay
///
/// Every fixture carries a NIP-40 `expiration` tag, and the local relay —
/// like any relay — refuses an expired event. Orders expire within a day
/// of publication, so a corpus captured on one date is gone from the relay
/// a day later, and an end-to-end test seeded with the events as captured
/// would lose an order a day until the pinned summary no longer matched.
/// The events are therefore rebuilt without the `expiration` tag and signed
/// with these keys: every tag, timestamp and identifier is the instance's
/// own, the signature verifies, and only the pubkey differs from the one
/// on the network. The unit tests keep the events exactly as captured.
fn test_keys(real: &PublicKey) -> Keys {
    // The real x-only pubkey is 32 random-looking bytes, which is what a
    // secret key is; the derivation needs no hash to be deterministic.
    Keys::new(SecretKey::from_slice(&real.to_bytes()).expect("a valid scalar"))
}

/// `event` as the local relay will keep it: without its `expiration`,
/// signed by [`test_keys`].
fn for_relay(event: &Event) -> Event {
    EventBuilder::new(event.kind, event.content.clone())
        .tags(
            event
                .tags
                .iter()
                .filter(|tag| tag.kind() != "expiration")
                .cloned(),
        )
        .custom_created_at(event.created_at)
        .finalize(&test_keys(&event.pubkey))
        .expect("sign")
}

/// Every fixture of the indexed kinds, other platforms included: the
/// pipeline has to turn those away, and a walk that never meets one would
/// not show that it does. Re-signed for the relay; see [`for_relay`].
fn fixtures() -> Vec<Event> {
    let mut events = Vec::new();
    for kind in KINDS {
        let dir = manifest_dir().join("tests/fixtures").join(kind.to_string());
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let json = fs::read_to_string(&path).expect("fixture");
            let captured = Event::from_json(&json).expect("a signed event");
            events.push(for_relay(&captured));
        }
    }
    events
}

/// A settings file pointing at the local relay and a fresh database.
///
/// Every network the fixtures use is indexed, so the one `success` order
/// in the corpus — published on regtest — is counted: a summary with no
/// completed order would leave the volume and completion rate untested.
fn write_settings(dir: &Path, relay_url: &str) -> PathBuf {
    let path = dir.join("settings.toml");
    let database = dir.join("e2e.db");
    fs::write(
        &path,
        format!(
            r#"
[nostr]
relays = ["{relay_url}"]

[indexer]
instances = []
accept_unknown_instances = true
networks = ["mainnet", "regtest"]
backfill_from = {FROM}

[database]
url = "sqlite://{}"
"#,
            database.display()
        ),
    )
    .expect("write settings");
    path
}

/// Runs the binary with `args`, asserting it exited zero, and returns its
/// stdout.
fn bestiario(settings: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_bestiario"))
        .arg("--config")
        .arg(settings)
        .args(args)
        .output()
        .expect("run bestiario");

    assert!(
        output.status.success(),
        "`bestiario {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("utf-8 stdout")
}

/// The JSON envelope of SPEC §10, checked for shape.
fn envelope(stdout: &str, command: &str) -> serde_json::Value {
    let json: serde_json::Value = serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("`{command}` did not print JSON: {e}\n{stdout}"));

    assert!(json["generated_at"].is_string(), "{command}: {json}");
    assert!(json["range"]["from"].is_string(), "{command}: {json}");
    assert!(json["range"]["until"].is_string(), "{command}: {json}");
    let metrics = json["metrics"]
        .as_array()
        .unwrap_or_else(|| panic!("{command}: metrics is not an array: {json}"));
    assert!(!metrics.is_empty(), "{command}: no metrics");
    for metric in metrics {
        assert!(metric["name"].is_string(), "{command}: {metric}");
        assert!(
            metric["kind"] == "observed" || metric["kind"] == "inferred",
            "{command}: {metric}"
        );
        assert!(metric.get("value").is_some(), "{command}: {metric}");
        if metric["kind"] == "inferred" {
            assert!(metric["error"].is_string(), "{command}: {metric}");
        }
    }

    json
}

#[tokio::test(flavor = "multi_thread")]
async fn backfill_then_every_report_against_the_local_relay() {
    // Arrange
    let relay = MockRelay::run().await.expect("start the local relay");
    let events = fixtures();
    for event in &events {
        relay.add_event(event.clone()).await.expect("seed");
    }
    let mostro = test_keys(&PublicKey::from_hex(MOSTRO).expect("hex")).public_key();
    let instance_prefix = &mostro.to_hex()[..8];
    let order_id = events
        .iter()
        .find(|event| event.kind.as_u16() == 38383 && event.pubkey == mostro)
        .and_then(|event| event.tags.identifier())
        .expect("an order from the profiled instance")
        .to_string();

    let dir = TempDir::new().expect("tempdir");
    let settings = write_settings(dir.path(), &relay.url().await.to_string());
    let dump_dir = std::env::var_os("E2E_DUMP_DIR").map(PathBuf::from);
    if let Some(dir) = &dump_dir {
        fs::create_dir_all(dir).expect("dump dir");
    }

    // Act: every command the README shows, in the README's order, against
    // this corpus — its `backfill` is the first walk, on an empty archive,
    // which is what the README's example has to show. A README that has
    // drifted from the binary fails here rather than misleading a reader.
    // The outputs are dumped as `readme-<n>.txt` so the examples in the
    // README can be pasted from the same run.
    for (index, command) in readme_commands().iter().enumerate() {
        let args: Vec<&str> = command.split_whitespace().collect();
        let stdout = bestiario(&settings, &args);
        if args.contains(&"--json") {
            envelope(&stdout, command);
        }
        if let Some(dir) = &dump_dir {
            fs::write(
                dir.join(format!("readme-{index:02}.txt")),
                format!("$ bestiario {command}\n{stdout}"),
            )
            .expect("dump");
        }
    }

    // Act: a second walk, from the binary's own JSON report.
    let backfill = bestiario(&settings, &["backfill", "--json"]);
    let backfill: serde_json::Value = serde_json::from_str(&backfill).expect("backfill json");

    // Assert: nothing new to store — the README's walk stored it all — the
    // whole corpus already known, and the other platforms turned away again.
    assert_eq!(backfill["events"]["stored"], 0, "{backfill}");
    assert!(
        backfill["events"]["duplicate"].as_u64().unwrap() > 0,
        "{backfill}"
    );
    assert!(
        backfill["events"]["rejected"].as_u64().unwrap() > 0,
        "{backfill}"
    );

    // Act / Assert: every report renders the envelope.
    let window = ["--from", FROM, "--until", UNTIL, "--json"];
    let reports: Vec<Vec<&str>> = vec![
        vec!["summary"],
        vec!["instances"],
        vec!["instance", instance_prefix],
        vec!["compare"],
        vec!["stats", "orders"],
        vec!["stats", "orders", "--by", "fiat"],
        vec!["stats", "orders", "--by", "period"],
        vec!["stats", "dev-fees"],
        vec!["stats", "disputes"],
        vec!["orders", &order_id],
    ];
    let mut summary = None;
    for report in reports {
        let mut args = report.clone();
        args.extend(window);
        let stdout = bestiario(&settings, &args);
        let json = envelope(&stdout, &report.join(" "));
        if report == ["summary"] {
            summary = Some(json);
        }

        if let Some(dir) = &dump_dir {
            let table = bestiario(&settings, &args[..args.len() - 1]);
            let name = report.join("-").replace(['/', ' '], "_");
            fs::write(dir.join(format!("{name}.txt")), table).expect("dump");
        }
    }

    // The summary is pinned exactly, clock excluded.
    let mut summary = summary.expect("summary ran");
    summary["generated_at"] = serde_json::Value::Null;
    let expected_path = manifest_dir().join("tests/expected/summary.json");
    let rendered = serde_json::to_string_pretty(&summary).expect("json") + "\n";
    if std::env::var_os("E2E_UPDATE").is_some() {
        fs::write(&expected_path, &rendered).expect("write expected");
    }
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e} — run with E2E_UPDATE=1",
            expected_path.display()
        )
    });
    assert_eq!(
        rendered, expected,
        "summary drifted from tests/expected/summary.json"
    );

    // And a rebuild reproduces the same summary from the archive.
    bestiario(&settings, &["rebuild", "--from-raw"]);
    let again = bestiario(
        &settings,
        &["summary", "--from", FROM, "--until", UNTIL, "--json"],
    );
    let mut again: serde_json::Value = serde_json::from_str(&again).expect("json");
    again["generated_at"] = serde_json::Value::Null;
    assert_eq!(again, summary, "rebuild changed the summary");
}

/// Every `$ bestiario …` line inside a fenced code block of `README.md`,
/// without the prompt and the binary name.
///
/// `sync` is left out: it runs until interrupted, which is the one thing a
/// test cannot wait for. The README shows it in a block without a prompt.
fn readme_commands() -> Vec<String> {
    let readme = fs::read_to_string(manifest_dir().join("README.md")).expect("README.md");
    let mut commands = Vec::new();
    let mut fenced = false;
    for line in readme.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if let Some(command) = line.strip_prefix("$ bestiario ").filter(|_| fenced) {
            commands.push(command.trim().to_string());
        }
    }
    assert!(
        !commands.is_empty(),
        "README.md shows no `$ bestiario` commands"
    );
    commands
}
