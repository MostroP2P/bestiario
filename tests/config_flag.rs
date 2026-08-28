//! `--config`, `docs/SPEC.md` §10: a missing configuration file is tolerated
//! only when the flag was omitted.
//!
//! The container deployment ships no `settings.toml` and configures the
//! binary entirely through `BESTIARIO__*`, so an absent file at the default
//! path has to be allowed. An operator who *names* a file has said the file
//! exists, and a typo there must fail rather than quietly index with
//! whatever the environment happens to hold — including when the name typed
//! is the default one.

use std::path::Path;
use std::process::{Command, Output};

/// Enough of an environment to pass validation: relays, and something to
/// index. The database lands inside `dir`, which is the process's working
/// directory, so nothing is written outside the test.
fn invoke_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bestiario"))
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("BESTIARIO__NOSTR__RELAYS", "wss://relay.mostro.network")
        .env("BESTIARIO__INDEXER__ACCEPT_UNKNOWN_INSTANCES", "true")
        .args(args)
        .output()
        .expect("run bestiario")
}

#[test]
fn an_explicitly_named_missing_file_fails_even_at_the_default_path() {
    let dir = tempfile::tempdir().expect("temp dir");

    let output = invoke_in(dir.path(), &["--config", "settings.toml", "summary"]);

    assert!(
        !output.status.success(),
        "a named file that does not exist must not fall back to the environment"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("loading settings.toml"),
        "the error should name the file that is missing, got:\n{stderr}"
    );
}

#[test]
fn an_omitted_flag_lets_the_environment_supply_everything() {
    let dir = tempfile::tempdir().expect("temp dir");

    let output = invoke_in(dir.path(), &["summary"]);

    // The report itself has nothing to say — the database it just created is
    // empty — and reaching that complaint is the point: configuration was
    // loaded from the environment alone, with no file anywhere.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("loading settings.toml"),
        "no settings.toml and no flag is the container deployment, got:\n{stderr}"
    );
    assert!(
        stderr.contains("run `bestiario backfill` first"),
        "expected the empty-database report, got:\n{stderr}"
    );
}
