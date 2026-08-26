//! Rebuilding, against a database filled by the pipeline itself.
//!
//! Every test ingests a corpus the normal way, snapshots what was derived,
//! destroys it and rebuilds — so what is asserted is that the two paths agree,
//! not that the rebuild matches a literal somebody typed out.

use sqlx::SqlitePool;

use super::*;
use crate::db::connect_and_migrate;
use crate::ingest::parse::fixtures::load;
use crate::network::Network;

const MEMORY: &str = "sqlite::memory:";
const RELAY: &str = "wss://relay.mostro.network";
const NOW: i64 = 1_787_800_000;

/// One of each indexed kind, plus a second version of the same order so the
/// projection has something to choose between.
const CORPUS: [(u16, &str); 6] = [
    (38383, "pending_range"),
    (38383, "canceled"),
    (38383, "in_progress"),
    (8383, "typical"),
    (38386, "status_settled"),
    (38385, "typical"),
];

/// What a rebuild has to reproduce exactly.
#[derive(Debug, PartialEq)]
struct Derived {
    orders: Vec<crate::db::repo::orders::Order>,
    disputes: Vec<crate::db::repo::disputes::Dispute>,
    instances: Vec<crate::db::repo::instances::Instance>,
    names: Vec<(String, Vec<(String, i64)>)>,
    dev_fees: Vec<crate::db::repo::dev_fees::StoredDevFee>,
}

async fn ingested() -> SqlitePool {
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");
    let pipeline = Pipeline::new(
        pool.clone(),
        Policy::new(Vec::<String>::new(), true, [Network::Mainnet]),
    );

    for (kind, name) in CORPUS {
        pipeline
            .ingest(&load(kind, name), RELAY, NOW)
            .await
            .expect("ingest");
    }

    pool
}

async fn derived(pool: &SqlitePool) -> Derived {
    let mut orders = Vec::new();
    for id in repo::orders::ids(pool).await.expect("order ids") {
        orders.push(
            repo::orders::find(pool, &id)
                .await
                .expect("read")
                .expect("projected"),
        );
    }

    let mut disputes = Vec::new();
    for id in repo::disputes::ids(pool).await.expect("dispute ids") {
        disputes.push(
            repo::disputes::find(pool, &id)
                .await
                .expect("read")
                .expect("projected"),
        );
    }

    let instances = repo::instances::all(pool).await.expect("instances");

    let mut names = Vec::new();
    for instance in &instances {
        names.push((
            instance.pubkey.clone(),
            repo::instances::names(pool, &instance.pubkey)
                .await
                .expect("names"),
        ));
    }

    let mut dev_fees = Vec::new();
    for id in repo::dev_fees::order_ids(pool).await.expect("fee orders") {
        dev_fees.extend(repo::dev_fees::for_order(pool, &id).await.expect("fees"));
    }

    Derived {
        orders,
        disputes,
        instances,
        names,
        dev_fees,
    }
}

async fn count(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(pool)
        .await
        .expect("count")
}

const ORDER_VERSIONS: &str = "SELECT COUNT(*) FROM order_versions";
const EVENTS: &str = "SELECT COUNT(*) FROM events";

#[tokio::test]
async fn wiping_every_projection_and_rebuilding_restores_all_four() {
    // Arrange
    let pool = ingested().await;
    let before = derived(&pool).await;
    repo::orders::clear_projection(&pool).await.expect("wipe");
    repo::disputes::clear_projection(&pool).await.expect("wipe");
    repo::instances::clear(&pool).await.expect("wipe");

    // Act
    let rebuilt = rebuild(&pool, false).await.expect("rebuild");

    // Assert
    assert_eq!(derived(&pool).await, before);
    assert_eq!(rebuilt.unreadable, 0);
    assert_eq!(rebuilt.events, CORPUS.len() as u64);
}

#[tokio::test]
async fn rebuilding_a_database_nothing_was_wiped_from_changes_nothing() {
    // Idempotence is what makes it safe to run after a crash: the operator
    // does not have to know how far the last attempt got.
    let pool = ingested().await;
    let before = derived(&pool).await;

    rebuild(&pool, false).await.expect("first");
    rebuild(&pool, false).await.expect("second");

    assert_eq!(derived(&pool).await, before);
}

#[tokio::test]
async fn from_raw_regenerates_the_version_tables_too() {
    // Arrange
    let pool = ingested().await;
    let before = derived(&pool).await;
    let versions_before = count(&pool, ORDER_VERSIONS).await;

    // Act
    let rebuilt = rebuild(&pool, true).await.expect("rebuild");

    // Assert
    assert_eq!(count(&pool, ORDER_VERSIONS).await, versions_before);
    assert_eq!(derived(&pool).await, before);
    assert_eq!(rebuilt.events, CORPUS.len() as u64);
}

#[tokio::test]
async fn a_plain_rebuild_leaves_the_versions_it_projects_from_alone() {
    // Arrange
    let pool = ingested().await;
    let versions_before = count(&pool, ORDER_VERSIONS).await;

    // Act
    rebuild(&pool, false).await.expect("rebuild");

    // Assert
    assert_eq!(count(&pool, ORDER_VERSIONS).await, versions_before);
}

#[tokio::test]
async fn an_order_whose_raw_event_is_no_longer_readable_is_still_projected() {
    // The sweep, not the replay, is what covers this. A foreign key keeps a
    // version from outliving its event, but nothing keeps an event from
    // becoming unreadable — a corrupt row, or a parser that grew stricter
    // than the corpus it was written against. The versions already stored are
    // still versions, and their projection is still owed.
    let pool = ingested().await;
    let before = derived(&pool).await;
    sqlx::query("UPDATE events SET raw_json = 'not json at all' WHERE kind = 38383")
        .execute(&pool)
        .await
        .expect("corrupt the raw orders");
    repo::orders::clear_projection(&pool).await.expect("wipe");

    rebuild(&pool, false).await.expect("rebuild");

    assert_eq!(derived(&pool).await.orders, before.orders);
}

#[tokio::test]
async fn an_unreadable_archived_event_is_counted_and_left_where_it_is() {
    // Arrange
    let pool = ingested().await;
    sqlx::query("UPDATE events SET raw_json = 'not json at all' WHERE kind = 8383")
        .execute(&pool)
        .await
        .expect("corrupt one row");
    let archived = count(&pool, EVENTS).await;

    // Act
    let rebuilt = rebuild(&pool, false).await.expect("rebuild");

    // Assert
    assert_eq!(rebuilt.unreadable, 1);
    assert_eq!(rebuilt.events, CORPUS.len() as u64 - 1);
    assert_eq!(count(&pool, EVENTS).await, archived);
}
