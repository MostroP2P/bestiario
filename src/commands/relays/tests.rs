use super::*;
use crate::db::connect_and_migrate;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const NOW: i64 = 1_787_800_000;

fn settings(discover: bool) -> Settings {
    let toml = format!(
        r#"
[nostr]
relays = ["wss://relay.mostro.network", "wss://nos.lol"]
discover_relays = {discover}

[indexer]
instances = ["{ALPHA}"]
networks = ["mainnet"]

[database]
url = "sqlite::memory:"
"#
    );
    Settings::from_toml_str(&toml).expect("valid settings")
}

async fn with_a_discovered_relay() -> SqlitePool {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");
    relays::upsert(
        &pool,
        "wss://discovered.example",
        &Source::Nip65 {
            pubkey: ALPHA.to_string(),
        },
        NOW - 100,
    )
    .await
    .expect("discovered");
    pool
}

#[tokio::test]
async fn with_discovery_off_the_set_is_exactly_what_was_configured() {
    // Arrange: an instance has advertised a relay, and the operator did not
    // ask to follow the advice.
    let pool = with_a_discovered_relay().await;

    // Act
    let set = connection_set(&pool, &settings(false), NOW)
        .await
        .expect("set");

    // Assert
    assert_eq!(set, ["wss://relay.mostro.network", "wss://nos.lol"]);
}

#[tokio::test]
async fn with_discovery_on_the_advertised_relays_follow_the_configured_ones() {
    let pool = with_a_discovered_relay().await;

    let set = connection_set(&pool, &settings(true), NOW)
        .await
        .expect("set");

    assert_eq!(
        set,
        [
            "wss://relay.mostro.network",
            "wss://nos.lol",
            "wss://discovered.example"
        ]
    );
}

#[tokio::test]
async fn a_relay_both_configured_and_advertised_is_dialled_once() {
    let pool = with_a_discovered_relay().await;
    relays::upsert(
        &pool,
        "wss://nos.lol",
        &Source::Nip65 {
            pubkey: ALPHA.to_string(),
        },
        NOW - 50,
    )
    .await
    .expect("advertised");

    let set = connection_set(&pool, &settings(true), NOW)
        .await
        .expect("set");

    assert_eq!(set.iter().filter(|url| *url == "wss://nos.lol").count(), 1);
}

#[tokio::test]
async fn the_configured_relays_are_recorded_as_configured() {
    let pool = connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate");

    connection_set(&pool, &settings(true), NOW)
        .await
        .expect("set");

    let known = relays::all(&pool).await.expect("all");
    assert_eq!(known.len(), 2);
    assert!(known.iter().all(|relay| relay.source == Source::Config));
    assert!(relays::discovered(&pool).await.expect("none").is_empty());
}
