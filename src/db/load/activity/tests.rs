//! The loader against a real migrated SQLite, seeded the way the pipeline
//! seeds it: an `events` row, a version, a projection refresh.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::db::repo::events::{self, EventRecord};
use crate::db::repo::{instances, orders};
use crate::ingest::parse::order::{FiatAmount, OrderVersion};
use crate::network::Network;
use crate::stats::activity::Origin;

const MEMORY: &str = "sqlite::memory:";
const ALPHA: &str = "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390";
const BETA: &str = "1b7b0f8d6c3e4a5f9e2d1c0b8a7f6e5d4c3b2a1f0e9d8c7b6a5f4e3d2c1b0a99";
const T0: i64 = 1_787_700_000;

async fn migrated() -> SqlitePool {
    connect_and_migrate(MEMORY).await.expect("migrate")
}

fn version(order_id: &str, pubkey: &str, created_at: i64, status: Status) -> OrderVersion {
    OrderVersion {
        event_id: format!("{order_id}-{created_at}"),
        order_id: order_id.to_string(),
        pubkey: pubkey.to_string(),
        created_at,
        direction: Direction::Sell,
        status,
        fiat_code: "VES".to_string(),
        amount_sats: 21_000,
        fiat: FiatAmount::Fixed(100.0),
        payment_methods: vec!["face to face".to_string(), "bank".to_string()],
        premium: 5.0,
        network: Some(Network::Mainnet),
        expires_at: created_at + 900,
    }
}

async fn ingest(pool: &SqlitePool, version: &OrderVersion) {
    let record = EventRecord {
        id: version.event_id.clone(),
        pubkey: version.pubkey.clone(),
        kind: 38383,
        created_at: version.created_at,
        d_tag: Some(version.order_id.clone()),
        raw_json: "{}".to_string(),
        relay_url: "wss://relay.mostro.network".to_string(),
        seen_at: version.created_at,
    };
    events::insert_if_new(pool, &record).await.expect("event");
    orders::insert_version(pool, version)
        .await
        .expect("version");
    orders::refresh_projection(pool, &version.order_id)
        .await
        .expect("refresh");
}

fn mainnet() -> Scope {
    Scope {
        pubkey: None,
        networks: vec![Network::Mainnet],
    }
}

#[tokio::test]
async fn a_completed_order_carries_its_whole_lifecycle() {
    // Arrange
    let pool = migrated().await;
    ingest(&pool, &version("o1", ALPHA, T0, Status::Pending)).await;
    ingest(&pool, &version("o1", ALPHA, T0 + 600, Status::InProgress)).await;
    ingest(&pool, &version("o1", ALPHA, T0 + 1_200, Status::Success)).await;
    instances::upsert(&pool, ALPHA, Some("Alpha"), T0)
        .await
        .expect("instance");

    // Act
    let loaded = orders(&pool, &mainnet()).await.expect("load");

    // Assert
    assert_eq!(
        loaded,
        vec![Order {
            order_id: "o1".to_string(),
            pubkey: ALPHA.to_string(),
            instance: "Alpha (82fa8cb9)".to_string(),
            created_at: T0,
            status: activity::Status::Success,
            direction: activity::Direction::Sell,
            fiat_code: "VES".to_string(),
            payment_methods: vec!["face to face".to_string(), "bank".to_string()],
            amount_sats: 21_000,
            fiat_amount: Some(100.0),
            premium: 5.0,
            is_market_price: false,
            fiat_range: None,
            pending_at: Some(T0),
            origin: Origin {
                fiat_code: "VES".to_string(),
                payment_methods: vec!["face to face".to_string(), "bank".to_string()],
                direction: crate::stats::activity::Direction::Sell,
            },
            taken_at: Some(T0 + 600),
            success_at: Some(T0 + 1_200),
            canceled_at: None,
            expires_at: Some(T0 + 1_200 + 900),
        }]
    );
}

#[tokio::test]
async fn an_instance_with_no_name_is_labelled_by_its_pubkey() {
    let pool = migrated().await;
    ingest(&pool, &version("o1", BETA, T0, Status::Pending)).await;
    instances::upsert(&pool, BETA, None, T0)
        .await
        .expect("instance");

    let loaded = orders(&pool, &mainnet()).await.expect("load");

    assert_eq!(loaded[0].instance, BETA);
    assert_eq!(loaded[0].taken_at, None, "never taken");
    assert_eq!(loaded[0].expires_at, Some(T0 + 900));
}

#[tokio::test]
async fn two_instances_sharing_a_name_keep_distinct_labels() {
    let pool = migrated().await;
    ingest(&pool, &version("o1", ALPHA, T0, Status::Pending)).await;
    ingest(&pool, &version("o2", BETA, T0 + 1, Status::Pending)).await;
    for pubkey in [ALPHA, BETA] {
        instances::upsert(&pool, pubkey, Some("Mostro"), T0)
            .await
            .expect("instance");
    }

    let loaded = orders(&pool, &mainnet()).await.expect("load");

    let labels: Vec<&str> = loaded.iter().map(|order| order.instance.as_str()).collect();
    assert_eq!(labels, vec!["Mostro (82fa8cb9)", "Mostro (1b7b0f8d)"]);
}

#[tokio::test]
async fn the_scope_narrows_to_one_instance() {
    let pool = migrated().await;
    ingest(&pool, &version("o1", ALPHA, T0, Status::Pending)).await;
    ingest(&pool, &version("o2", BETA, T0 + 1, Status::Pending)).await;

    let scope = Scope {
        pubkey: Some(BETA.to_string()),
        ..mainnet()
    };
    let loaded = orders(&pool, &scope).await.expect("load");

    let ids: Vec<&str> = loaded.iter().map(|order| order.order_id.as_str()).collect();
    assert_eq!(ids, vec!["o2"]);
}

#[tokio::test]
async fn the_scope_narrows_to_the_listed_networks() {
    let pool = migrated().await;
    ingest(&pool, &version("o1", ALPHA, T0, Status::Pending)).await;
    ingest(
        &pool,
        &OrderVersion {
            network: Some(Network::Testnet),
            ..version("o2", ALPHA, T0 + 1, Status::Pending)
        },
    )
    .await;

    let mainnet_only = orders(&pool, &mainnet()).await.expect("load");
    let any = orders(&pool, &Scope::default()).await.expect("load");

    assert_eq!(mainnet_only.len(), 1);
    assert_eq!(any.len(), 2, "an empty network list is no filter");
}

#[tokio::test]
async fn orders_come_back_oldest_first() {
    let pool = migrated().await;
    ingest(&pool, &version("late", ALPHA, T0 + 100, Status::Pending)).await;
    ingest(&pool, &version("early", ALPHA, T0, Status::Pending)).await;

    let loaded = orders(&pool, &mainnet()).await.expect("load");

    let ids: Vec<&str> = loaded.iter().map(|order| order.order_id.as_str()).collect();
    assert_eq!(ids, vec!["early", "late"]);
}

#[tokio::test]
async fn completed_in_reads_only_the_orders_settled_inside_the_bounds() {
    // Arrange: settled before, inside, and at the upper bound; one open.
    let pool = migrated().await;
    for (id, at) in [
        ("before", T0 - 10),
        ("inside", T0 + 10),
        ("at_until", T0 + 100),
    ] {
        ingest(&pool, &version(id, ALPHA, at - 5, Status::Pending)).await;
        ingest(&pool, &version(id, ALPHA, at, Status::Success)).await;
    }
    ingest(&pool, &version("open", ALPHA, T0 + 20, Status::Pending)).await;

    // Act
    let orders = completed_in(&pool, &mainnet(), T0, T0 + 100)
        .await
        .expect("load");

    // Assert
    let ids: Vec<&str> = orders.iter().map(|order| order.order_id.as_str()).collect();
    assert_eq!(ids, ["inside"]);
    assert_eq!(orders[0].success_at, Some(T0 + 10));
}

#[tokio::test]
async fn price_type_and_range_come_from_the_first_version() {
    // Arrange: a market-price range order — `amt = 0`, `fa = [10, 100]` —
    // taken and settled at 21 000 sats for a fixed 50.
    let pool = migrated().await;
    ingest(
        &pool,
        &OrderVersion {
            amount_sats: 0,
            fiat: FiatAmount::Range {
                min: 10.0,
                max: 100.0,
            },
            ..version("o1", ALPHA, T0, Status::Pending)
        },
    )
    .await;
    ingest(
        &pool,
        &OrderVersion {
            fiat: FiatAmount::Fixed(50.0),
            ..version("o1", ALPHA, T0 + 600, Status::Success)
        },
    )
    .await;

    // Act
    let orders = orders(&pool, &mainnet()).await.expect("load");

    // Assert
    assert!(orders[0].is_market_price);
    assert_eq!(orders[0].fiat_range, Some((10.0, 100.0)));
    assert_eq!(
        orders[0].amount_sats, 21_000,
        "sats from the latest version"
    );
    assert_eq!(orders[0].fiat_amount, Some(50.0));
}

#[tokio::test]
async fn an_order_first_seen_mid_flight_has_no_pending_anchor_and_its_origin_is_that_version() {
    // Caught already in progress: no pending version to date the book entry.
    let pool = migrated().await;
    ingest(&pool, &version("o1", ALPHA, T0, Status::InProgress)).await;
    ingest(&pool, &version("o1", ALPHA, T0 + 60, Status::Success)).await;

    let orders = orders(&pool, &mainnet()).await.expect("load");

    assert_eq!(orders[0].pending_at, None);
    assert_eq!(orders[0].taken_at, Some(T0));
    assert_eq!(orders[0].origin.fiat_code, "VES");
}

#[tokio::test]
async fn lifecycle_in_reads_what_moved_in_the_window_and_the_live_book() {
    // Arrange: `before` settled before the window; `moved` was created
    // before and settled inside; `live` is pending from before and still
    // open; `stale` is pending from before and expired.
    let pool = migrated().await;
    ingest(&pool, &version("before", ALPHA, T0 - 100, Status::Pending)).await;
    ingest(&pool, &version("before", ALPHA, T0 - 50, Status::Success)).await;
    ingest(&pool, &version("moved", ALPHA, T0 - 100, Status::Pending)).await;
    ingest(&pool, &version("moved", ALPHA, T0 + 50, Status::Success)).await;
    ingest(
        &pool,
        &OrderVersion {
            expires_at: T0 + 10_000,
            ..version("live", ALPHA, T0 - 100, Status::Pending)
        },
    )
    .await;
    ingest(
        &pool,
        &OrderVersion {
            expires_at: T0 - 10,
            ..version("stale", ALPHA, T0 - 100, Status::Pending)
        },
    )
    .await;

    // Act
    let orders = lifecycle_in(&pool, &mainnet(), T0, T0 + 100, T0 + 200)
        .await
        .expect("load");

    // Assert
    let mut ids: Vec<&str> = orders.iter().map(|order| order.order_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["live", "moved"]);
}
#[tokio::test]
async fn the_payment_methods_of_the_book_come_from_the_first_version() {
    // Arrange: an order published with `cash` that later advertises `pix`
    // as well. §6.3 counts what was on the book, §6.1 what it says now.
    let pool = migrated().await;
    ingest(
        &pool,
        &OrderVersion {
            payment_methods: vec!["cash".to_string()],
            ..version("o1", ALPHA, T0, Status::Pending)
        },
    )
    .await;
    ingest(
        &pool,
        &OrderVersion {
            payment_methods: vec!["cash".to_string(), "pix".to_string()],
            ..version("o1", ALPHA, T0 + 600, Status::Success)
        },
    )
    .await;

    // Act
    let orders = orders(&pool, &mainnet()).await.expect("load");

    // Assert
    assert_eq!(orders[0].origin.payment_methods, vec!["cash".to_string()]);
    assert_eq!(
        orders[0].payment_methods,
        vec!["cash".to_string(), "pix".to_string()]
    );
}
