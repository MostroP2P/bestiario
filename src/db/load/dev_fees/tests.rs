//! The loader against a real migrated SQLite, seeded the way the pipeline
//! seeds it.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{dev_fees, instance_info, instances, orders};
use crate::ingest::parse::dev_fee::DevFee;
use crate::ingest::parse::info::InstanceInfo;
use crate::ingest::parse::order::{Direction, FiatAmount, OrderVersion, Status};
use crate::network::Network;

const MEMORY: &str = "sqlite::memory:";
const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";
const T0: i64 = 1_787_700_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

async fn event(pool: &SqlitePool, id: &str, pubkey: &str, kind: u16, created_at: i64) {
    let record = EventRecord {
        id: id.to_string(),
        pubkey: pubkey.to_string(),
        kind: kind.into(),
        created_at,
        d_tag: None,
        raw_json: "{}".to_string(),
        relay_url: "wss://relay.mostro.network".to_string(),
        seen_at: created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
}

async fn order(pool: &SqlitePool, order_id: &str, pubkey: &str, created_at: i64, status: Status) {
    let version = OrderVersion {
        event_id: format!("{order_id}-{created_at}"),
        order_id: order_id.to_string(),
        pubkey: pubkey.to_string(),
        created_at,
        direction: Direction::Sell,
        status,
        fiat_code: "VES".to_string(),
        amount_sats: 21_000,
        fiat: FiatAmount::Fixed(100.0),
        payment_methods: vec!["cash".to_string()],
        premium: 0.0,
        network: Some(Network::Mainnet),
        expires_at: created_at + 900,
    };
    event(pool, &version.event_id, pubkey, 38383, created_at).await;
    orders::insert_version(pool, &version)
        .await
        .expect("version");
    orders::refresh_projection(pool, order_id)
        .await
        .expect("refresh");
}

async fn fee(pool: &SqlitePool, id: &str, order_id: &str, pubkey: &str, created_at: i64) {
    let fee = DevFee {
        event_id: id.to_string(),
        pubkey: pubkey.to_string(),
        order_id: order_id.to_string(),
        created_at,
        amount_sats: 100,
        payment_hash: format!("hash-{id}"),
        destination: None,
        network: Some(Network::Mainnet),
    };
    event(pool, id, pubkey, 8383, created_at).await;
    dev_fees::insert(pool, &fee).await.expect("fee");
}

async fn info(pool: &SqlitePool, pubkey: &str, created_at: i64, fee: Option<f64>) {
    let id = format!("info-{pubkey}-{created_at}");
    event(pool, &id, pubkey, 38385, created_at).await;
    instance_info::insert_version(
        pool,
        &InstanceInfo {
            event_id: id,
            pubkey: pubkey.to_string(),
            created_at,
            fee,
            max_order_amount: None,
            min_order_amount: None,
            fiat_currencies: None,
            mostro_version: None,
            protocol_version: None,
            ln_networks: None,
            bond_enabled: None,
        },
    )
    .await
    .expect("info");
}

fn mainnet() -> Scope {
    Scope {
        pubkey: None,
        networks: vec![Network::Mainnet],
    }
}

#[tokio::test]
async fn a_fee_carries_what_the_projection_knows_about_its_order() {
    // Arrange
    let pool = migrated().await;
    order(&pool, "o1", ALPHA, T0, Status::Success).await;
    fee(&pool, "f1", "o1", ALPHA, T0 + 60).await;
    fee(&pool, "f2", "unseen", ALPHA, T0 + 120).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), T0)
        .await
        .expect("instance");

    // Act
    let data = load(&pool, &mainnet()).await.expect("load");

    // Assert
    assert_eq!(
        data.fees,
        vec![
            Fee {
                event_id: "f1".into(),
                order_id: "o1".into(),
                pubkey: ALPHA.into(),
                instance: "Alpha (82fa8cb9)".into(),
                created_at: T0 + 60,
                amount_sats: 100,
                is_duplicate: false,
                order_known: true,
                settled_at: Some(T0),
                fee_in_force: None,
                order_amount_sats: Some(21_000),
            },
            Fee {
                event_id: "f2".into(),
                order_id: "unseen".into(),
                pubkey: ALPHA.into(),
                instance: "Alpha (82fa8cb9)".into(),
                created_at: T0 + 120,
                amount_sats: 100,
                is_duplicate: false,
                order_known: false,
                settled_at: None,
                fee_in_force: None,
                order_amount_sats: None,
            },
        ]
    );
}

#[tokio::test]
async fn a_fee_for_an_order_that_did_not_complete_has_no_settlement_time() {
    let pool = migrated().await;
    order(&pool, "o1", ALPHA, T0, Status::Canceled).await;
    fee(&pool, "f1", "o1", ALPHA, T0 + 60).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    assert!(data.fees[0].order_known);
    assert_eq!(data.fees[0].settled_at, None);
}

#[tokio::test]
async fn the_second_fee_for_an_order_is_flagged_as_a_duplicate() {
    let pool = migrated().await;
    fee(&pool, "f-late", "o1", ALPHA, T0 + 120).await;
    fee(&pool, "f-early", "o1", ALPHA, T0 + 60).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    let flags: Vec<(&str, bool)> = data
        .fees
        .iter()
        .map(|fee| (fee.event_id.as_str(), fee.is_duplicate))
        .collect();
    assert_eq!(flags, vec![("f-early", false), ("f-late", true)]);
}

#[tokio::test]
async fn a_settlement_says_whether_it_was_paid_for_and_whether_it_owed_anything() {
    let pool = migrated().await;
    info(&pool, ALPHA, T0 - 100, Some(0.006)).await;
    info(&pool, BETA, T0 - 100, Some(0.0)).await;
    order(&pool, "paid", ALPHA, T0, Status::Success).await;
    fee(&pool, "f1", "paid", ALPHA, T0 + 60).await;
    order(&pool, "unpaid", ALPHA, T0 + 1, Status::Success).await;
    order(&pool, "free", BETA, T0 + 2, Status::Success).await;
    order(&pool, "open", ALPHA, T0 + 3, Status::Pending).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    let seen: Vec<(&str, bool, Option<bool>)> = data
        .settlements
        .iter()
        .map(|s| (s.order_id.as_str(), s.has_fee, s.charges_fee))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("paid", true, Some(true)),
            ("unpaid", false, Some(true)),
            ("free", false, Some(false)),
        ]
    );
}

#[tokio::test]
async fn the_fee_in_force_is_the_one_at_settlement_not_the_newest() {
    let pool = migrated().await;
    info(&pool, ALPHA, T0 - 100, Some(0.0)).await;
    order(&pool, "o1", ALPHA, T0, Status::Success).await;
    // Raised afterwards: the trade settled under the zero fee.
    info(&pool, ALPHA, T0 + 100, Some(0.01)).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    assert_eq!(data.settlements[0].charges_fee, Some(false));
}

#[tokio::test]
async fn an_instance_that_never_published_info_has_an_unknown_fee() {
    let pool = migrated().await;
    order(&pool, "o1", ALPHA, T0, Status::Success).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    assert_eq!(data.settlements[0].charges_fee, None);
}

#[tokio::test]
async fn the_scope_narrows_both_reads() {
    let pool = migrated().await;
    order(&pool, "oa", ALPHA, T0, Status::Success).await;
    order(&pool, "ob", BETA, T0 + 1, Status::Success).await;
    fee(&pool, "fa", "oa", ALPHA, T0 + 60).await;
    fee(&pool, "fb", "ob", BETA, T0 + 61).await;

    let scope = Scope {
        pubkey: Some(BETA.to_string()),
        ..mainnet()
    };
    let data = load(&pool, &scope).await.expect("load");

    assert_eq!(data.fees.len(), 1);
    assert_eq!(data.fees[0].event_id, "fb");
    assert_eq!(data.settlements.len(), 1);
    assert_eq!(data.settlements[0].order_id, "ob");
}

#[tokio::test]
async fn a_fee_carries_the_fee_in_force_when_its_order_settled_or_when_it_was_paid() {
    // Arrange: 0.6% until just after o1 settles, 1% from then on. The fee
    // for o1 is paid under the new rate but the trade was under the old;
    // the orphan has no trade to date it by, so its own timestamp does.
    let pool = migrated().await;
    info(&pool, ALPHA, T0 - 100, Some(0.006)).await;
    order(&pool, "o1", ALPHA, T0, Status::Success).await;
    info(&pool, ALPHA, T0 + 10, Some(0.01)).await;
    fee(&pool, "f1", "o1", ALPHA, T0 + 60).await;
    fee(&pool, "f2", "unseen", ALPHA, T0 + 120).await;

    // Act
    let data = load(&pool, &mainnet()).await.expect("load");

    // Assert
    assert_eq!(data.fees[0].fee_in_force, Some(0.006));
    assert_eq!(data.fees[1].fee_in_force, Some(0.01));
}
