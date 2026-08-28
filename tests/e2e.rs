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
//! # The README examples
//!
//! Every `$ bestiario …` line inside a fenced block of `README.md` is run,
//! and the output shown under it in the block has to be what the binary
//! printed — byte for byte, the clock being frozen (`BESTIARIO_NOW`). A
//! README that has drifted from the binary fails here rather than
//! misleading a reader. `E2E_DUMP_DIR=<dir> cargo test --test e2e` writes
//! every output, the README commands' as `readme-<n>.txt`, so the examples
//! can be pasted back from the same run.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nostr_sdk::prelude::{
    Client, Event, EventBuilder, Filter, FinalizeEvent as _, Keys, Kind, MockRelay, PublicKey,
    SecretKey, Tag,
};
use tempfile::TempDir;

/// The kinds the backfill walks, in the fixture directories of the same
/// name. Relay lists (10002) are discovery, not indexing.
const KINDS: [u16; 5] = [38383, 8383, 38386, 38385, 30078];

/// A window around the capture date of the fixtures (2026-08-26).
const FROM: &str = "1787500000";
const UNTIL: &str = "1787800000";

/// The clock the binary runs on (`BESTIARIO_NOW`): 2026-08-27T03:06:40Z,
/// just after the window. Frozen so that the reports — `open_now`,
/// `silent_for`, `generated_at` — are the same on every run and the
/// README's examples can be compared byte for byte.
const NOW: &str = "1787800000";

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
///
/// A kind 38385 is keyed on its own pubkey (`d` = pubkey hex, SPEC §2.4),
/// and the parser checks that; the `d` follows the key.
fn for_relay(event: &Event) -> Event {
    let keys = test_keys(&event.pubkey);
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

/// The key the E2E publisher signs with: a throwaway, generated once, and
/// the only key in this repository. It signs nothing but the documents a
/// local relay holds for the length of one test.
///
/// It reaches the binary through the environment of the child process, the
/// way a real one does — never through the settings file, which refuses to
/// hold a key at all.
const PUBLISHER_NSEC: &str = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";

/// A clock one second past [`NOW`], for the one publication that has
/// changed figures to store: see `invoke_at`.
const LATER: &str = "1787800001";

/// The variable `[publish].nsec` points at, here as anywhere.
const PUBLISHER_NSEC_VAR: &str = "BESTIARIO_PUBLISH_NSEC";

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

[publish]
nsec = "env:{PUBLISHER_NSEC_VAR}"
"#,
            database.display()
        ),
    )
    .expect("write settings");
    path
}

/// Runs the binary with `args`, and with the signing key exported into its
/// environment or deliberately absent from it.
///
/// The environment is the child's alone: a test that exported the key into
/// its own process would export it into every test running beside it.
fn invoke(settings: &Path, args: &[&str], key_exported: bool) -> std::process::Output {
    invoke_at(settings, args, key_exported, NOW)
}

/// The same, with the run's clock given explicitly.
///
/// Kind 30666 is addressable, so a relay keeps one event per
/// `(pubkey, kind, d)` and refuses a replacement whose `created_at` is not
/// later than the one it holds. Two publications of *changed* figures in
/// the same second are therefore not both storable — which the rest of
/// this suite never notices, because it publishes the same bytes twice.
fn invoke_at(
    settings: &Path,
    args: &[&str],
    key_exported: bool,
    now: &str,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bestiario"));
    command
        .env("BESTIARIO_NOW", now)
        .env_remove(PUBLISHER_NSEC_VAR);
    if key_exported {
        command.env(PUBLISHER_NSEC_VAR, PUBLISHER_NSEC);
    }
    command
        .arg("--config")
        .arg(settings)
        .args(args)
        .output()
        .expect("run bestiario")
}

/// Runs the binary with `args`, asserting it exited zero, and returns its
/// stdout.
fn bestiario(settings: &Path, args: &[&str]) -> String {
    let output = invoke(settings, args, true);

    assert!(
        output.status.success(),
        "`bestiario {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("utf-8 stdout")
}

/// Runs the binary with `args`, asserting it exited non-zero, and returns
/// its stderr — for the refusals that are the point of the invocation.
fn bestiario_refuses(settings: &Path, args: &[&str], key_exported: bool) -> String {
    let output = invoke(settings, args, key_exported);

    assert!(
        !output.status.success(),
        "`bestiario {}` was expected to fail and did not:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout)
    );

    String::from_utf8(output.stderr).expect("utf-8 stderr")
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
    // Update mode: regenerate the pinned outputs instead of checking them.
    let updating = std::env::var_os("E2E_UPDATE").is_some();
    if let Some(dir) = &dump_dir {
        fs::create_dir_all(dir).expect("dump dir");
    }

    // Act: every command the README shows, in the README's order, against
    // this corpus — its `backfill` is the first walk, on an empty archive,
    // which is what the README's example has to show. A README that has
    // drifted from the binary fails here rather than misleading a reader.
    // The outputs are dumped as `readme-<n>.txt` so the examples in the
    // README can be pasted from the same run.
    for (index, (command, shown)) in readme_examples().iter().enumerate() {
        let args = shell_words(command);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
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
        if !updating {
            assert_eq!(
                stdout.trim_end(),
                shown.trim_end(),
                "the README's example for `bestiario {command}` is not what the binary prints; \
                 run with E2E_UPDATE=1 E2E_DUMP_DIR=<dir> and paste readme-{index:02}.txt"
            );
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
    if updating {
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

    // Act: publish the snapshot for real — signed, to the relay the
    // fixtures came from. `--dry-run` has already run above, as one of the
    // README's examples; this is the half of §12 that a key turns on.
    let report = bestiario(&settings, &["publish"]);

    // Assert: every document the run listed is on the relay, under the
    // publisher's key and the addressable kind of §2.
    let published = relay_documents(&relay).await;
    let publisher = Keys::parse(PUBLISHER_NSEC)
        .expect("the E2E key")
        .public_key();
    assert!(
        published.iter().all(|event| event.pubkey == publisher),
        "something on the relay was not signed by the configured key"
    );
    let addresses: BTreeSet<String> = published
        .iter()
        .filter_map(|event| event.tags.identifier())
        .collect();
    assert!(
        addresses.contains("index"),
        "the index of §5 was never published: {addresses:?}"
    );
    assert!(
        addresses.contains("orders:30d") && addresses.contains("series:volume:monthly:2026"),
        "a window document and a series partition should both be there: {addresses:?}"
    );
    assert_eq!(
        addresses.len(),
        published.len(),
        "a `d` address was published twice in one run"
    );
    assert!(
        report.contains("index last"),
        "the run has to say it published the index last: {report}"
    );

    // And the snapshot the run computed is the snapshot the relay holds:
    // every `s` tag names this run, which is what lets a client ask for a
    // whole publication in one filter (§7).
    // Act / Assert: a second run over an unchanged archive re-sends no
    // document (§8). Every figure is the one already published, and a
    // relay does not need a second copy of an answer that did not
    // change. The index goes out anyway — nothing hashes it and naming
    // the current snapshot is its whole job (§5).
    let again = bestiario(&settings, &["publish"]);
    assert!(
        again.contains("0 document(s) sent"),
        "a second run over an unchanged archive re-signed figures: {again}"
    );
    assert_eq!(
        relay_documents(&relay).await.len(),
        published.len(),
        "the second run added events for figures that did not move"
    );

    // But `--republish` is the recovery path for a relay that lost them,
    // so it distrusts exactly that assumption (§9.3). The documents come
    // back with the same revisions: re-signing an unchanged payload is
    // not a restatement.
    let recovered = bestiario(&settings, &["publish", "--republish"]);
    assert!(
        recovered.contains("index last"),
        "--republish has to send the whole snapshot: {recovered}"
    );
    let after = relay_documents(&relay).await;
    assert_eq!(
        after.len(),
        published.len(),
        "an addressable kind keeps one event per `d`, so a republication replaces rather than adds"
    );
    let revisions: BTreeSet<&str> = after
        .iter()
        .filter_map(|event| {
            event
                .tags
                .iter()
                .find(|tag| tag.kind() == "revision")
                .and_then(Tag::content)
        })
        .collect();
    assert_eq!(
        revisions,
        BTreeSet::from(["1"]),
        "re-signing an unchanged payload is not a restatement (§9.3)"
    );

    // Act: a figure moves. An order the archive did not hold reaches the
    // relay, a backfill stores it, and the next publication has something
    // to restate.
    let restated = {
        let seed = events
            .iter()
            .find(|event| event.kind.as_u16() == 38383 && event.pubkey == mostro)
            .expect("an order from the profiled instance");
        let keys = test_keys(&PublicKey::from_hex(MOSTRO).expect("hex"));
        let tags = seed.tags.iter().map(|tag| {
            if tag.kind() == "d" {
                Tag::identifier("00000000-0000-4000-8000-00000000e2e1")
            } else {
                tag.clone()
            }
        });
        let extra = EventBuilder::new(seed.kind, seed.content.clone())
            .tags(tags)
            .custom_created_at(seed.created_at)
            .finalize(&keys)
            .expect("sign");
        relay.add_event(extra).await.expect("seed one more order");

        bestiario(&settings, &["backfill"]);
        // A second later than every publication above: see `invoke_at`.
        let output = invoke_at(&settings, &["publish"], true, LATER);
        assert!(
            output.status.success(),
            "`bestiario publish` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8 stdout")
    };

    // Assert: the documents whose figures moved are at revision 2, and say
    // why (§8). The reason is read off the archive — the run was never
    // told a backfill had happened — and a backfill is what reaching an
    // order the archive did not hold is.
    assert!(
        restated.contains("index last"),
        "figures moved, so the snapshot is published: {restated}"
    );
    let moved: Vec<Event> = relay_documents(&relay)
        .await
        .into_iter()
        .filter(|event| {
            event
                .tags
                .iter()
                .find(|tag| tag.kind() == "revision")
                .and_then(Tag::content)
                == Some("2")
        })
        .collect();
    assert!(
        !moved.is_empty(),
        "an order nobody had counted moved no figure at all"
    );
    for event in &moved {
        let envelope: serde_json::Value =
            serde_json::from_str(&event.content).expect("an envelope");
        assert_eq!(
            envelope["restated_because"], "backfill",
            "a revision above the first says why (§8): {envelope}"
        );
        assert!(envelope["restated_at"].is_string(), "and when: {envelope}");
    }

    // And the two ways of having no key are told apart. Configuring none
    // at all is one run that would otherwise read the whole archive and
    // send nothing.
    let keyless = dir.path().join("keyless.toml");
    fs::write(
        &keyless,
        fs::read_to_string(&settings)
            .expect("settings")
            .replace(&format!("nsec = \"env:{PUBLISHER_NSEC_VAR}\""), ""),
    )
    .expect("write keyless settings");
    let refusal = bestiario_refuses(&keyless, &["publish"], true);
    assert!(
        refusal.contains("no signing key"),
        "a keyless run has to say why it did nothing: {refusal}"
    );

    // Naming a variable nobody exported is the other, and it names the
    // variable — which is the only thing that says what to export.
    let refusal = bestiario_refuses(&settings, &["publish"], false);
    assert!(
        refusal.contains(PUBLISHER_NSEC_VAR),
        "an unexported key has to name the variable: {refusal}"
    );

    // But only a run that is going to sign asks for the key at all: a
    // review is the invocation of somebody who does not hold one, on a
    // machine that is not the publisher.
    let reviewed = invoke(&settings, &["publish", "--dry-run"], false);
    assert!(
        reviewed.status.success(),
        "--dry-run asked for a key it never uses:\n{}",
        String::from_utf8_lossy(&reviewed.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&reviewed.stdout).contains("index last"),
        "--dry-run signs nothing and publishes nothing"
    );

    let runs: BTreeSet<&str> = published
        .iter()
        .filter_map(|event| {
            event
                .tags
                .iter()
                .find(|tag| tag.kind() == "s")
                .and_then(Tag::content)
        })
        .collect();
    assert_eq!(runs.len(), 1, "one publication, one snapshot_id: {runs:?}");
}

/// Every kind 30666 document the relay is holding.
async fn relay_documents(relay: &MockRelay) -> Vec<Event> {
    let client = Client::default();
    client.add_relay(relay.url().await).await.expect("add");
    client.connect().await;
    let events = client
        .fetch_events(Filter::new().kind(Kind::from_u16(30666)))
        .await
        .expect("read the documents back");
    client.shutdown().await;
    events.into_iter().collect()
}

/// `line` split the way a shell would, as far as double quotes: an
/// instance name with a space in it is written `"Mostro Brasil"` in the
/// README, and has to reach the binary as one argument.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut pending = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                pending = true;
            }
            c if c.is_whitespace() && !quoted => {
                if pending {
                    words.push(std::mem::take(&mut word));
                    pending = false;
                }
            }
            c => {
                word.push(c);
                pending = true;
            }
        }
    }
    if pending {
        words.push(word);
    }
    words
}

/// Every `$ bestiario …` line inside a fenced code block of `README.md`,
/// without the prompt and the binary name, paired with the output the
/// block shows under it (empty when the block shows none).
///
/// `sync` is left out: it runs until interrupted, which is the one thing a
/// test cannot wait for. The README shows it in a block without a prompt.
fn readme_examples() -> Vec<(String, String)> {
    let readme = fs::read_to_string(manifest_dir().join("README.md")).expect("README.md");
    let mut examples: Vec<(String, String)> = Vec::new();
    let mut fenced = false;
    let mut current: Option<(String, String)> = None;
    for line in readme.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            if let Some(example) = current.take() {
                examples.push(example);
            }
            continue;
        }
        if !fenced {
            continue;
        }
        if let Some(command) = line.strip_prefix("$ bestiario ") {
            if let Some(example) = current.take() {
                examples.push(example);
            }
            current = Some((command.trim().to_string(), String::new()));
        } else if let Some((_, shown)) = &mut current {
            shown.push_str(line);
            shown.push('\n');
        }
    }
    assert!(
        !examples.is_empty(),
        "README.md shows no `$ bestiario` commands"
    );
    examples
}
