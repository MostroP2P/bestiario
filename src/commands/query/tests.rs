//! Resolving the global flags into a [`Query`]: window, instance, networks.

use clap::Parser as _;
use sqlx::SqlitePool;

use super::*;
use crate::cli::Cli;
use crate::commands::Context;
use crate::config::Settings;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};

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

/// One stored event, so that the database is not empty.
async fn seed_event(pool: &SqlitePool) {
    events::insert_if_new(
        pool,
        &EventRecord {
            id: "e".to_string(),
            pubkey: ALPHA.to_string(),
            kind: 38385,
            created_at: NOW - 1,
            d_tag: Some(ALPHA.to_string()),
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: NOW - 1,
        },
    )
    .await
    .expect("event");
}

/// Resolves `args` against a database holding one named instance and one
/// event.
async fn resolve(args: &[&str]) -> Result<Query> {
    let settings = Settings::from_toml_str(SETTINGS).expect("valid settings");
    let pool = migrated().await;
    seed_event(&pool).await;
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
    assert!(!query.network_narrowed);
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
    assert!(query.network_narrowed);
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

#[tokio::test]
async fn a_database_with_no_events_refuses_to_report_and_says_what_to_run() {
    // Arrange: migrated, never backfilled — a table of zeros would read as
    // an answer, and it is not one.
    let settings = Settings::from_toml_str(SETTINGS).expect("valid settings");
    let pool = migrated().await;
    let cli = cli(&["stats", "orders"]);

    // Act
    let error = Query::resolve(
        &Context {
            settings: &settings,
            pool: &pool,
            cli: &cli,
        },
        NOW,
    )
    .await
    .expect_err("an empty database is refused");

    // Assert
    let message = error.to_string();
    assert!(message.contains("no events"), "{message}");
    assert!(message.contains("bestiario backfill"), "{message}");
}
