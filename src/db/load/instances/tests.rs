use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instance_info, instances};
use crate::ingest::parse::info::InstanceInfo;

const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";
const T0: i64 = 1_787_700_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate("sqlite::memory:")
        .await
        .expect("migrate")
}

async fn info(pool: &SqlitePool, pubkey: &str, created_at: i64, version: &str, fee: Option<f64>) {
    sparse_info(pool, pubkey, created_at, Some(version), fee, true).await;
}

/// A 38385 carrying only what is given: `version`, `fee`, and the rest of
/// the profile only when `full`.
async fn sparse_info(
    pool: &SqlitePool,
    pubkey: &str,
    created_at: i64,
    version: Option<&str>,
    fee: Option<f64>,
    full: bool,
) {
    let id = format!("info-{pubkey}-{created_at}");
    events::insert_if_new(
        pool,
        &EventRecord {
            id: id.clone(),
            pubkey: pubkey.to_string(),
            kind: 38385,
            created_at,
            d_tag: None,
            raw_json: "{}".to_string(),
            relay_url: "wss://relay.mostro.network".to_string(),
            seen_at: created_at,
        },
    )
    .await
    .expect("event");
    instance_info::insert_version(
        pool,
        &InstanceInfo {
            event_id: id,
            pubkey: pubkey.to_string(),
            created_at,
            fee,
            max_order_amount: full.then_some(500_000),
            min_order_amount: full.then_some(1_000),
            fiat_currencies: full.then(|| "ARS,VES".to_string()),
            mostro_version: version.map(str::to_string),
            protocol_version: full.then(|| "1".to_string()),
            ln_networks: full.then(|| "mainnet".to_string()),
            bond_enabled: full.then_some(true),
        },
    )
    .await
    .expect("info");
}

#[tokio::test]
async fn a_profile_is_the_instance_row_plus_its_latest_info() {
    // Arrange
    let pool = migrated().await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), T0)
        .await
        .expect("instance");
    instances::upsert(&pool, ALPHA, None, T0 + 500)
        .await
        .expect("instance");
    info(&pool, ALPHA, T0, "0.13.0", Some(0.005)).await;
    info(&pool, ALPHA, T0 + 100, "0.14.0", Some(0.006)).await;

    // Act
    let profiles = profiles(&pool, &Scope::default()).await.expect("load");

    // Assert
    assert_eq!(
        profiles,
        vec![Profile {
            pubkey: ALPHA.into(),
            name: Some("Alpha".into()),
            label: "Alpha (82fa8cb9)".into(),
            mostro_version: Some("0.14.0".into()),
            protocol_version: Some("1".into()),
            fee: Some(0.006),
            min_order_sats: Some(1_000),
            max_order_sats: Some(500_000),
            fiat_currencies: vec!["ARS".into(), "VES".into()],
            ln_networks: vec!["mainnet".into()],
            bond_enabled: Some(true),
            first_seen_at: T0,
            last_seen_at: T0 + 500,
        }]
    );
}

#[tokio::test]
async fn a_newer_sparse_info_does_not_erase_what_an_older_one_published() {
    // The latest event carries only a version: the fee and the limits are
    // still the ones the instance last stated.
    let pool = migrated().await;
    instances::upsert(&pool, ALPHA, None, T0)
        .await
        .expect("instance");
    info(&pool, ALPHA, T0, "0.13.0", Some(0.005)).await;
    sparse_info(&pool, ALPHA, T0 + 100, Some("0.14.0"), None, false).await;

    let profiles = profiles(&pool, &Scope::default()).await.expect("load");

    assert_eq!(profiles[0].mostro_version.as_deref(), Some("0.14.0"));
    assert_eq!(profiles[0].fee, Some(0.005));
    assert_eq!(profiles[0].min_order_sats, Some(1_000));
    assert_eq!(profiles[0].fiat_currencies, vec!["ARS", "VES"]);
}

#[tokio::test]
async fn an_instance_that_never_published_info_still_has_a_profile() {
    let pool = migrated().await;
    instances::upsert(&pool, BETA, None, T0)
        .await
        .expect("instance");

    let profiles = profiles(&pool, &Scope::default()).await.expect("load");

    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].label, BETA);
    assert_eq!(profiles[0].mostro_version, None);
    assert_eq!(profiles[0].fiat_currencies, Vec::<String>::new());
}

#[tokio::test]
async fn the_scope_narrows_to_one_instance_and_ignores_networks() {
    let pool = migrated().await;
    instances::upsert(&pool, ALPHA, None, T0)
        .await
        .expect("instance");
    instances::upsert(&pool, BETA, None, T0 + 1)
        .await
        .expect("instance");

    let scope = Scope {
        pubkey: Some(BETA.to_string()),
        networks: vec![crate::network::Network::Testnet],
    };
    let profiles = profiles(&pool, &scope).await.expect("load");

    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].pubkey, BETA);
}
