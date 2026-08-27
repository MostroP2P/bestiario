use super::*;
use crate::db::connect_and_migrate;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const T0: i64 = 1_787_700_000;

async fn migrated() -> sqlx::SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

#[tokio::test]
async fn a_relay_keeps_the_source_that_named_it_first() {
    // Arrange: the operator configured it; an instance advertises it later.
    let pool = migrated().await;
    upsert(&pool, "wss://relay.example", &Source::Config, T0)
        .await
        .expect("config");

    // Act
    upsert(
        &pool,
        "wss://relay.example",
        &Source::Nip65 {
            pubkey: ALPHA.to_string(),
        },
        T0 + 100,
    )
    .await
    .expect("nip65");

    // Assert
    let relays = all(&pool).await.expect("all");
    assert_eq!(relays.len(), 1, "one URL is one relay");
    assert_eq!(relays[0].source, Source::Config);
    assert_eq!(relays[0].first_seen_at, T0);
}

#[tokio::test]
async fn a_discovered_relay_names_the_instance_that_advertised_it() {
    let pool = migrated().await;
    let source = Source::Nip65 {
        pubkey: ALPHA.to_string(),
    };
    upsert(&pool, "wss://discovered.example", &source, T0)
        .await
        .expect("nip65");
    upsert(&pool, "wss://configured.example", &Source::Config, T0 + 1)
        .await
        .expect("config");

    let discovered = discovered(&pool).await.expect("discovered");

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].url, "wss://discovered.example");
    assert_eq!(discovered[0].source, source);
}

#[test]
fn a_source_survives_the_round_trip_through_the_column() {
    let nip65 = Source::Nip65 {
        pubkey: ALPHA.to_string(),
    };

    assert_eq!(nip65.as_stored(), format!("nip65:{ALPHA}"));
    assert_eq!(Source::parse(&nip65.as_stored()), nip65);
    assert_eq!(Source::Config.as_stored(), "config");
    assert_eq!(Source::parse("config"), Source::Config);
}

#[tokio::test]
async fn nothing_stored_is_no_relays() {
    let pool = migrated().await;

    assert!(all(&pool).await.expect("all").is_empty());
    assert!(discovered(&pool).await.expect("discovered").is_empty());
}
