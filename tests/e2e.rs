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

use nostr_sdk::prelude::{Event, MockRelay};
use tempfile::TempDir;

/// The kinds the backfill walks, in the fixture directories of the same
/// name. Rates and relay lists are phase 3 and 4.
const KINDS: [u16; 4] = [38383, 8383, 38386, 38385];

/// A window around the capture date of the fixtures (2026-08-26).
const FROM: &str = "1787500000";
const UNTIL: &str = "1787800000";

/// Enough of `82fa8cb9…` (the instance named "Mostro") to be unique.
const INSTANCE_PREFIX: &str = "82fa8cb9";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every fixture of the indexed kinds, other platforms included: the
/// pipeline has to turn those away, and a walk that never meets one would
/// not show that it does.
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
            events.push(Event::from_json(&json).expect("a signed event"));
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
    let order_id = events
        .iter()
        .find(|event| {
            event.kind.as_u16() == 38383 && event.pubkey.to_hex().starts_with(INSTANCE_PREFIX)
        })
        .and_then(|event| event.tags.identifier())
        .expect("an order from the profiled instance")
        .to_string();

    let dir = TempDir::new().expect("tempdir");
    let settings = write_settings(dir.path(), &relay.url().await.to_string());

    // Act: the walk.
    let backfill = bestiario(&settings, &["backfill", "--json"]);
    let backfill: serde_json::Value = serde_json::from_str(&backfill).expect("backfill json");

    // Assert: everything Mostro was stored, everything else turned away.
    assert!(
        backfill["events"]["stored"].as_u64().unwrap() > 0,
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
        vec!["instance", INSTANCE_PREFIX],
        vec!["compare"],
        vec!["stats", "orders"],
        vec!["stats", "orders", "--by", "fiat"],
        vec!["stats", "orders", "--by", "period"],
        vec!["stats", "dev-fees"],
        vec!["stats", "disputes"],
        vec!["orders", &order_id],
    ];
    let dump_dir = std::env::var_os("E2E_DUMP_DIR").map(PathBuf::from);
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
            fs::create_dir_all(dir).expect("dump dir");
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
