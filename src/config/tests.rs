//! One test per validation rule of [`super::Settings`], plus the round trip
//! through a real file and the environment layer.

use std::collections::BTreeMap;

use super::*;
use crate::network::Network;

/// A configuration with every rule satisfied. Tests mutate one line of it to
/// exercise one rule, so a failure points at the rule and not at the fixture.
const VALID: &str = r#"
[nostr]
relays = ["wss://relay.mostro.network", "wss://nos.lol"]
discover_relays = true
resume_overlap_secs = 1800

[indexer]
instances = ["82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390"]
accept_unknown_instances = false
networks = ["mainnet"]
backfill_from = 1735689600

[assumptions]
dev_fee_percentage_default = 0.30

[database]
url = "sqlite://bestiario.db"

[report]
reference_currency = "USD"
"#;

/// Replaces the single line starting with `key` — enough to break exactly one
/// rule per test without restating the whole file.
fn with_line(key: &str, replacement: &str) -> String {
    VALID
        .lines()
        .map(|line| {
            if line.trim_start().starts_with(key) {
                replacement
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn expect_invalid(toml: &str) -> ValidationError {
    match Settings::from_toml_str(toml) {
        Err(ConfigError::Invalid(error)) => error,
        Err(other) => panic!("expected a validation error, got a load error: {other}"),
        Ok(_) => panic!("expected a validation error, but the settings were accepted"),
    }
}

#[test]
fn parses_a_fully_specified_file() {
    // Arrange / Act
    let settings = Settings::from_toml_str(VALID).expect("valid settings");

    // Assert
    assert_eq!(settings.nostr.relays.len(), 2);
    assert!(settings.nostr.discover_relays);
    assert_eq!(settings.nostr.resume_overlap_secs, 1800);
    assert_eq!(
        settings.indexer.instances,
        ["82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390"]
    );
    assert!(!settings.indexer.accept_unknown_instances);
    assert_eq!(settings.indexer.networks, [Network::Mainnet]);
    assert_eq!(settings.indexer.backfill_from, 1_735_689_600);
    assert_eq!(settings.assumptions.dev_fee_percentage_default, 0.30);
    assert_eq!(settings.database.url, "sqlite://bestiario.db");
    assert_eq!(settings.report.reference_currency, "USD");
}

#[test]
fn applies_documented_defaults_when_optional_sections_are_absent() {
    // Arrange
    let minimal = r#"
[nostr]
relays = ["wss://relay.mostro.network"]

[indexer]
instances = ["82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390"]

[database]
url = "sqlite://bestiario.db"
"#;

    // Act
    let settings = Settings::from_toml_str(minimal).expect("valid settings");

    // Assert
    assert_eq!(settings.nostr.resume_overlap_secs, 3600);
    assert!(!settings.nostr.discover_relays);
    assert_eq!(settings.indexer.networks, [Network::Mainnet]);
    assert_eq!(settings.indexer.backfill_from, 0);
    assert_eq!(settings.assumptions.dev_fee_percentage_default, 0.30);
    assert!(settings.assumptions.dev_fee_percentage.is_empty());
    assert_eq!(settings.report.reference_currency, "USD");
}

#[test]
fn rejects_an_unknown_key_rather_than_ignoring_it() {
    // A silently ignored typo would look exactly like a setting that does not
    // work, so unknown keys are an error.
    let toml = format!("{VALID}\n[nostr.typo]\nrelays = []\n");

    let error = Settings::from_toml_str(&toml).expect_err("unknown key");

    assert!(matches!(error, ConfigError::Load(_)), "got {error:?}");
}

#[test]
fn rejects_an_empty_relay_list() {
    let error = expect_invalid(&with_line("relays", "relays = []"));

    assert_eq!(error, ValidationError::NoRelays);
    assert_eq!(
        error.to_string(),
        "[nostr].relays is empty: at least one relay is required"
    );
}

#[test]
fn rejects_a_relay_that_is_not_a_websocket_url() {
    let error = expect_invalid(&with_line(
        "relays",
        r#"relays = ["https://relay.mostro.network"]"#,
    ));

    assert_eq!(
        error,
        ValidationError::RelayNotWebsocket {
            url: "https://relay.mostro.network".to_string()
        }
    );
}

#[test]
fn accepts_a_plaintext_relay_for_local_testing() {
    // The E2E suite of docs/SPEC.md §12 runs against a local relay, which is
    // not served over TLS.
    let settings =
        Settings::from_toml_str(&with_line("relays", r#"relays = ["ws://127.0.0.1:8080"]"#))
            .expect("ws:// is allowed");

    assert_eq!(settings.nostr.relays, ["ws://127.0.0.1:8080"]);
}

#[test]
fn rejects_a_pubkey_of_the_wrong_length() {
    let error = expect_invalid(&with_line("instances", r#"instances = ["82fa8cb9"]"#));

    assert_eq!(
        error,
        ValidationError::PubkeyLength {
            pubkey: "82fa8cb9".to_string(),
            len: 8
        }
    );
}

#[test]
fn rejects_a_pubkey_that_is_not_hexadecimal() {
    let pubkey = format!("z{}", "0".repeat(63));
    let error = expect_invalid(&with_line(
        "instances",
        &format!(r#"instances = ["{pubkey}"]"#),
    ));

    assert_eq!(error, ValidationError::PubkeyNotHex { pubkey, found: 'z' });
}

#[test]
fn accepts_an_uppercase_pubkey_and_folds_it_to_lowercase() {
    // Relays report pubkeys in lowercase hex; folding here means every later
    // comparison is plain string equality.
    let settings = Settings::from_toml_str(&with_line(
        "instances",
        r#"instances = ["82FA8CB978B43C79B2156585BAC2C011176A21D2AEAD6D9F7C575C005BE88390"]"#,
    ))
    .expect("uppercase hex is accepted");

    assert_eq!(
        settings.indexer.instances,
        ["82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390"]
    );
}

#[test]
fn rejects_a_configuration_that_would_index_nothing() {
    // No instances and no willingness to discover them is not a valid
    // indexer; it is a silent no-op.
    let error = expect_invalid(&with_line("instances", "instances = []"));

    assert_eq!(error, ValidationError::NothingToIndex);
}

#[test]
fn accepts_an_empty_instance_list_when_unknown_instances_are_welcome() {
    let toml = with_line("instances", "instances = []");
    let toml = toml.replace(
        "accept_unknown_instances = false",
        "accept_unknown_instances = true",
    );

    let settings = Settings::from_toml_str(&toml).expect("discovery mode is valid");

    assert!(settings.indexer.instances.is_empty());
    assert!(settings.indexer.accept_unknown_instances);
}

#[test]
fn rejects_an_empty_network_list() {
    let error = expect_invalid(&with_line("networks", "networks = []"));

    assert_eq!(error, ValidationError::NoNetworks);
}

#[test]
fn rejects_a_misspelled_network() {
    // `mainet` would filter out every event and report zeros, which is the
    // failure mode this rule exists to prevent. The vocabulary now lives in
    // the Network type, so this is caught by deserialization rather than by a
    // validation rule of its own.
    let error = Settings::from_toml_str(&with_line("networks", r#"networks = ["mainet"]"#))
        .expect_err("misspelled network");

    assert!(matches!(error, ConfigError::Load(_)), "got {error:?}");

    // Rendered the way the binary renders a fatal error: `{:#}` walks the
    // whole chain, which is where the offending value lives.
    let message = format!("{:#}", anyhow::Error::from(error));
    assert!(
        message.contains("mainet"),
        "should name the typo: {message}"
    );
}

#[test]
fn accepts_every_network_the_wire_format_can_carry() {
    let settings = Settings::from_toml_str(&with_line(
        "networks",
        r#"networks = ["mainnet", "testnet", "signet", "regtest"]"#,
    ))
    .expect("all four are valid");

    assert_eq!(
        settings.indexer.networks,
        [
            Network::Mainnet,
            Network::Testnet,
            Network::Signet,
            Network::Regtest
        ]
    );
}

#[test]
fn rejects_a_negative_backfill_timestamp() {
    let error = expect_invalid(&with_line("backfill_from", "backfill_from = -1"));

    assert_eq!(error, ValidationError::NegativeBackfillFrom { value: -1 });
}

#[test]
fn rejects_a_dev_fee_share_outside_the_unit_interval() {
    for value in ["0.0", "-0.1", "1.5"] {
        let error = expect_invalid(&with_line(
            "dev_fee_percentage_default",
            &format!("dev_fee_percentage_default = {value}"),
        ));

        assert!(
            matches!(error, ValidationError::DevFeePercentageOutOfRange { .. }),
            "{value} should have been rejected, got {error:?}"
        );
    }
}

#[test]
fn rejects_a_per_instance_dev_fee_share_outside_the_unit_interval() {
    let pubkey = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
    let toml = format!("{VALID}\n[assumptions.dev_fee_percentage]\n\"{pubkey}\" = 1.5\n");

    let error = expect_invalid(&toml);

    assert_eq!(
        error,
        ValidationError::DevFeePercentageOutOfRange {
            setting: format!("[assumptions.dev_fee_percentage].\"{pubkey}\""),
            value: 1.5
        }
    );
}

#[test]
fn rejects_a_per_instance_override_keyed_by_a_malformed_pubkey() {
    let toml = format!("{VALID}\n[assumptions.dev_fee_percentage]\n\"not-a-pubkey\" = 0.5\n");

    let error = expect_invalid(&toml);

    assert!(
        matches!(error, ValidationError::PubkeyLength { .. }),
        "got {error:?}"
    );
}

#[test]
fn rejects_a_database_url_that_is_not_sqlite() {
    let error = expect_invalid(&with_line(
        "url",
        r#"url = "postgres://localhost/bestiario""#,
    ));

    assert_eq!(
        error,
        ValidationError::DatabaseNotSqlite {
            url: "postgres://localhost/bestiario".to_string()
        }
    );
}

#[test]
fn rejects_a_reference_currency_that_is_not_a_three_letter_code() {
    for code in ["US", "DOLLAR", "US1"] {
        let error = expect_invalid(&with_line(
            "reference_currency",
            &format!(r#"reference_currency = "{code}""#),
        ));

        assert!(
            matches!(error, ValidationError::ReferenceCurrencyNotIso { .. }),
            "{code} should have been rejected, got {error:?}"
        );
    }
}

#[test]
fn folds_the_reference_currency_to_uppercase() {
    let settings = Settings::from_toml_str(&with_line(
        "reference_currency",
        r#"reference_currency = "usd""#,
    ))
    .expect("lowercase is accepted");

    assert_eq!(settings.report.reference_currency, "USD");
}

#[test]
fn dev_fee_percentage_falls_back_to_the_default_for_an_instance_without_an_override() {
    // Arrange
    let overridden = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
    let other = "1".repeat(64);
    let toml = format!("{VALID}\n[assumptions.dev_fee_percentage]\n\"{overridden}\" = 0.5\n");
    let settings = Settings::from_toml_str(&toml).expect("valid settings");

    // Act / Assert
    assert_eq!(settings.dev_fee_percentage_for(overridden), 0.5);
    assert_eq!(settings.dev_fee_percentage_for(&other), 0.30);
}

#[test]
fn dev_fee_percentage_lookup_is_case_insensitive() {
    let pubkey = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
    let toml = format!("{VALID}\n[assumptions.dev_fee_percentage]\n\"{pubkey}\" = 0.5\n");
    let settings = Settings::from_toml_str(&toml).expect("valid settings");

    assert_eq!(settings.dev_fee_percentage_for(&pubkey.to_uppercase()), 0.5);
}

/// Serializes the tests that call [`Settings::load`], because that is the one
/// entry point that reads process-wide environment state.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn loads_from_a_file_on_disk() {
    // Arrange
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("settings.toml");
    std::fs::write(&path, VALID).expect("write settings");

    // Act
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let settings = Settings::load(&path).expect("valid settings");

    // Assert
    assert_eq!(settings.database.url, "sqlite://bestiario.db");
}

#[test]
fn an_environment_variable_overrides_the_file() {
    // Arrange
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("settings.toml");
    std::fs::write(&path, VALID).expect("write settings");

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: every test that reads the environment holds ENV_LOCK, so no
    // other thread is reading it while this one writes.
    unsafe { std::env::set_var("BESTIARIO__DATABASE__URL", "sqlite://override.db") };

    // Act
    let settings = Settings::load(&path);

    // SAFETY: as above. Removed before the assertion so a failure cannot leak
    // the variable into the rest of the run.
    unsafe { std::env::remove_var("BESTIARIO__DATABASE__URL") };

    // Assert
    assert_eq!(
        settings.expect("valid settings").database.url,
        "sqlite://override.db"
    );
}

#[test]
fn an_environment_override_is_validated_like_any_other_value() {
    // Arrange
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("settings.toml");
    std::fs::write(&path, VALID).expect("write settings");

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: see `an_environment_variable_overrides_the_file`.
    unsafe { std::env::set_var("BESTIARIO__DATABASE__URL", "postgres://localhost/x") };

    // Act
    let result = Settings::load(&path);

    // SAFETY: as above.
    unsafe { std::env::remove_var("BESTIARIO__DATABASE__URL") };

    // Assert
    assert!(
        matches!(
            result,
            Err(ConfigError::Invalid(
                ValidationError::DatabaseNotSqlite { .. }
            ))
        ),
        "an override must not bypass validation"
    );
}

#[test]
fn reports_a_missing_file_rather_than_falling_back_to_defaults() {
    let dir = tempfile::tempdir().expect("temp dir");

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let error = Settings::load(&dir.path().join("absent.toml")).expect_err("missing file");

    assert!(matches!(error, ConfigError::Load(_)), "got {error:?}");
}

#[test]
fn the_shipped_example_is_a_valid_configuration() {
    // If the example drifts out of validity, every new operator's first run
    // fails. Cheap to check here.
    let example = include_str!("../../settings.toml.example");
    let settings = Settings::from_toml_str(example).expect("settings.toml.example is valid");

    assert!(!settings.nostr.relays.is_empty());
}

#[test]
fn per_instance_overrides_are_ordered_deterministically() {
    // BTreeMap rather than HashMap so that error messages and any future
    // rendering of the assumptions are reproducible.
    let a = "0".repeat(64);
    let b = "1".repeat(64);
    let toml = format!("{VALID}\n[assumptions.dev_fee_percentage]\n\"{b}\" = 0.5\n\"{a}\" = 0.4\n");
    let settings = Settings::from_toml_str(&toml).expect("valid settings");

    let keys: Vec<_> = settings.assumptions.dev_fee_percentage.keys().collect();
    assert_eq!(keys, vec![&a, &b]);
    let _: &BTreeMap<String, f64> = &settings.assumptions.dev_fee_percentage;
}
