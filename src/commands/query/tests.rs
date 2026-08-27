//! Resolving the global flags into a [`Query`]: window, instance, networks.

use clap::Parser as _;
use sqlx::SqlitePool;

use super::*;
use crate::cli::Cli;
use crate::commands::Context;
use crate::config::Settings;
use crate::db::connect_and_migrate;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const NOW: i64 = 1_787_800_000;

const SETTINGS: &str = r#"
[nostr]
relays = ["wss://relay.mostro.network"]

[indexer]
instances = ["82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390"]
networks = ["mainnet", "signet"]

[database]
url = "sqlite::memory:"
"#;

async fn migrated() -> SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

fn cli(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("bestiario").chain(args.iter().copied()))
        .expect("the invocation parses")
}

/// Resolves `args` against a database holding one named instance.
async fn resolve(args: &[&str]) -> Result<Query> {
    let settings = Settings::from_toml_str(SETTINGS).expect("valid settings");
    let pool = migrated().await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), NOW - 1)
        .await
        .expect("instance");
    let cli = cli(args);

    Query::resolve(
        &Context {
            settings: &settings,
            pool: &pool,
            cli: &cli,
        },
        NOW,
    )
    .await
}

#[tokio::test]
async fn without_flags_the_query_covers_every_configured_network_and_every_instance() {
    // Arrange / Act
    let query = resolve(&["stats", "orders"]).await.expect("resolves");

    // Assert
    assert_eq!(query.scope.pubkey, None);
    assert_eq!(
        query.scope.networks,
        vec![Network::Mainnet, Network::Signet]
    );
    assert_eq!(
        query.range,
        Range::resolve(None, None, NOW).expect("window")
    );
}

#[tokio::test]
async fn the_network_flag_overrides_the_configured_list() {
    let query = resolve(&["--network", "testnet", "stats", "orders"])
        .await
        .expect("resolves");

    assert_eq!(query.scope.networks, vec![Network::Testnet]);
}

#[tokio::test]
async fn the_instance_flag_is_resolved_by_name_to_a_pubkey() {
    let query = resolve(&["--instance", "alpha", "stats", "orders"])
        .await
        .expect("resolves");

    assert_eq!(query.scope.pubkey.as_deref(), Some(ALPHA));
}

#[tokio::test]
async fn an_unknown_instance_fails_before_anything_is_read() {
    let error = resolve(&["--instance", "nobody", "stats", "orders"])
        .await
        .expect_err("unknown instance");

    assert!(error.to_string().contains("nobody"), "{error}");
}

#[tokio::test]
async fn the_window_flags_become_the_range() {
    let query = resolve(&["--from", "1000", "--until", "2000", "stats", "orders"])
        .await
        .expect("resolves");

    assert_eq!(query.range.from(), 1_000);
    assert_eq!(query.range.until(), 2_000);
}
