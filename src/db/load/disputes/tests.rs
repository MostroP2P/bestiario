//! The loader against a real migrated SQLite.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{disputes as disputes_repo, instances, orders};
use crate::ingest::parse::dispute::DisputeVersion;
use crate::ingest::parse::order::{self, Direction, FiatAmount, OrderVersion};
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

async fn dispute(
    pool: &SqlitePool,
    id: &str,
    pubkey: &str,
    created_at: i64,
    status: Status,
    initiator: Option<Initiator>,
    opened_at: Option<i64>,
) {
    let version = DisputeVersion {
        event_id: format!("{id}-{created_at}"),
        dispute_id: id.to_string(),
        pubkey: pubkey.to_string(),
        created_at,
        status,
        initiator,
        opened_at,
    };
    event(pool, &version.event_id, pubkey, 38386, created_at).await;
    disputes_repo::insert_version(pool, &version)
        .await
        .expect("version");
    disputes_repo::refresh_projection(pool, id)
        .await
        .expect("refresh");
}

async fn order(pool: &SqlitePool, id: &str, pubkey: &str, created_at: i64, status: order::Status) {
    let version = OrderVersion {
        event_id: format!("{id}-{created_at}"),
        order_id: id.to_string(),
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
    orders::refresh_projection(pool, id).await.expect("refresh");
}

fn mainnet() -> Scope {
    Scope {
        pubkey: None,
        networks: vec![Network::Mainnet],
    }
}

#[tokio::test]
async fn a_dispute_carries_its_opening_and_its_resolution() {
    // Arrange
    let pool = migrated().await;
    dispute(
        &pool,
        "d1",
        ALPHA,
        T0 + 10,
        Status::Initiated,
        Some(Initiator::Buyer),
        Some(T0),
    )
    .await;
    dispute(
        &pool,
        "d1",
        ALPHA,
        T0 + 500,
        Status::Settled,
        Some(Initiator::Buyer),
        Some(T0),
    )
    .await;
    dispute(&pool, "d1", ALPHA, T0 + 600, Status::Settled, None, None).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), T0)
        .await
        .expect("instance");

    // Act
    let data = load(&pool, &mainnet()).await.expect("load");

    // Assert
    assert_eq!(
        data.disputes,
        vec![Dispute {
            dispute_id: "d1".into(),
            instance: "Alpha (82fa8cb9)".into(),
            opened_at: T0,
            status: disputes::Status::Settled,
            initiator: Some(disputes::Initiator::Buyer),
            resolved_at: Some(T0 + 500),
            outcome: Some(disputes::Status::Settled),
        }]
    );
}

#[tokio::test]
async fn a_dispute_that_never_published_its_opening_is_dated_by_its_first_version() {
    let pool = migrated().await;
    dispute(&pool, "d1", ALPHA, T0 + 10, Status::Initiated, None, None).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    assert_eq!(data.disputes[0].opened_at, T0 + 10);
    assert_eq!(data.disputes[0].initiator, None);
    assert_eq!(data.disputes[0].resolved_at, None);
}

#[tokio::test]
async fn taken_orders_are_dated_by_the_taker_or_by_the_settlement_when_none_was_seen() {
    let pool = migrated().await;
    order(&pool, "taken", ALPHA, T0, order::Status::Pending).await;
    order(&pool, "taken", ALPHA, T0 + 100, order::Status::InProgress).await;
    order(&pool, "taken", ALPHA, T0 + 200, order::Status::Success).await;
    // Backfill missed the in-progress version.
    order(&pool, "settled", ALPHA, T0 + 300, order::Status::Success).await;
    order(&pool, "pending", ALPHA, T0 + 400, order::Status::Pending).await;
    order(&pool, "abandoned", ALPHA, T0 + 500, order::Status::Canceled).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    let seen: Vec<(&str, i64)> = data
        .taken
        .iter()
        .map(|t| (t.order_id.as_str(), t.left_pending_at))
        .collect();
    assert_eq!(seen, vec![("taken", T0 + 100), ("settled", T0 + 300)]);
}

#[tokio::test]
async fn the_outcome_is_the_first_terminal_version_even_when_a_later_one_differs() {
    let pool = migrated().await;
    dispute(&pool, "d1", ALPHA, T0, Status::Initiated, None, None).await;
    dispute(&pool, "d1", ALPHA, T0 + 100, Status::Settled, None, None).await;
    dispute(&pool, "d1", ALPHA, T0 + 200, Status::Released, None, None).await;

    let data = load(&pool, &mainnet()).await.expect("load");

    assert_eq!(data.disputes[0].status, disputes::Status::Released);
    assert_eq!(data.disputes[0].outcome, Some(disputes::Status::Settled));
    assert_eq!(data.disputes[0].resolved_at, Some(T0 + 100));
}

#[tokio::test]
async fn the_network_scope_reaches_neither_read() {
    // Disputes carry no network; filtering only the orders would divide
    // every network's disputes by one network's takers.
    let pool = migrated().await;
    dispute(&pool, "d", ALPHA, T0, Status::Initiated, None, None).await;
    order(&pool, "o", ALPHA, T0, order::Status::Success).await;

    let scope = Scope {
        pubkey: None,
        networks: vec![Network::Testnet],
    };
    let data = load(&pool, &scope).await.expect("load");

    assert_eq!(data.disputes.len(), 1);
    assert_eq!(data.taken.len(), 1);
}

#[tokio::test]
async fn the_instance_scope_narrows_both_reads() {
    let pool = migrated().await;
    dispute(&pool, "da", ALPHA, T0, Status::Initiated, None, None).await;
    dispute(&pool, "db", BETA, T0 + 1, Status::Initiated, None, None).await;
    order(&pool, "oa", ALPHA, T0, order::Status::Success).await;
    order(&pool, "ob", BETA, T0 + 1, order::Status::Success).await;

    let scope = Scope {
        pubkey: Some(BETA.to_string()),
        ..mainnet()
    };
    let data = load(&pool, &scope).await.expect("load");

    assert_eq!(data.disputes.len(), 1);
    assert_eq!(data.disputes[0].dispute_id, "db");
    assert_eq!(data.taken.len(), 1);
    assert_eq!(data.taken[0].order_id, "ob");
}
