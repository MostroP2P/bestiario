//! Migration and connection tests. Every one of these runs against a real
//! SQLite database, in memory or in a temporary file — none of it is mocked,
//! because what is being tested is precisely what SQLite does with the schema.

use super::*;

/// The tables of `docs/SPEC.md` §4. Listed explicitly rather than counted, so
/// that a migration which drops one fails with the name of what went missing.
const EXPECTED_TABLES: [&str; 13] = [
    "dev_fees",
    "dispute_versions",
    "disputes",
    "events",
    "indexed_kinds",
    "instance_info",
    "instance_names",
    "instances",
    "order_versions",
    "orders",
    "rates",
    "relays",
    "sync_state",
];

const MEMORY: &str = "sqlite::memory:";

async fn table_names(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx%'
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("read sqlite_master")
}

#[tokio::test]
async fn migrating_creates_every_table_in_the_spec() {
    // Arrange / Act
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");

    // Assert
    assert_eq!(table_names(&pool).await, EXPECTED_TABLES);
}

#[tokio::test]
async fn migrating_creates_the_indexes_the_queries_will_need() {
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");

    let indexes = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master
         WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("read sqlite_master");

    // The three that carry the hot paths: lifecycle reconstruction, the
    // dev-fee join, and per-instance status counts.
    for expected in [
        "order_versions_order",
        "dev_fees_order",
        "orders_pubkey_status",
    ] {
        assert!(
            indexes.iter().any(|i| i == expected),
            "missing index {expected}; have {indexes:?}"
        );
    }
}

#[tokio::test]
async fn migrating_twice_is_a_no_op() {
    // Arrange
    let pool = connect_and_migrate(MEMORY).await.expect("first migrate");
    let before = table_names(&pool).await;

    // Act
    migrate(&pool).await.expect("second migrate");

    // Assert
    assert_eq!(table_names(&pool).await, before);
}

#[tokio::test]
async fn reopening_an_existing_database_preserves_its_contents() {
    // The indexer is a long-lived archive; the one thing a startup path must
    // never do is start over.
    let dir = tempfile::tempdir().expect("temp dir");
    let url = format!("sqlite://{}", dir.path().join("bestiario.db").display());

    let pool = connect_and_migrate(&url).await.expect("first open");
    sqlx::query("INSERT INTO relays (url, source, first_seen_at) VALUES (?, ?, ?)")
        .bind("wss://relay.mostro.network")
        .bind("config")
        .bind(1_735_689_600_i64)
        .execute(&pool)
        .await
        .expect("insert");
    pool.close().await;

    let pool = connect_and_migrate(&url).await.expect("reopen");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relays")
        .fetch_one(&pool)
        .await
        .expect("count");

    assert_eq!(count, 1);
}

#[tokio::test]
async fn creates_the_database_file_when_it_does_not_exist() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("new.db");
    let url = format!("sqlite://{}", path.display());

    let _pool = connect(&url).await.expect("connect");

    assert!(path.exists(), "expected {} to be created", path.display());
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    // SQLite leaves foreign keys off by default, so this is a real setting and
    // not a tautology: without `foreign_keys(true)` the insert below succeeds.
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");

    let result = sqlx::query(
        "INSERT INTO rates (event_id, pubkey, published_at, source, rates_json)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind("an-event-id-that-was-never-stored")
    .bind("82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390")
    .bind(1_735_689_600_i64)
    .bind("yadio")
    .bind("{}")
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "a row referencing an absent event should have been rejected"
    );
}

#[tokio::test]
async fn a_dev_fee_may_name_an_order_that_has_not_been_seen() {
    // The retention asymmetry of docs/SPEC.md §2.2: an 8383 outlives its
    // 38383, so orphan dev fees are normal input, not corruption. This is why
    // dev_fees.order_id carries no foreign key.
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");

    sqlx::query(
        "INSERT INTO events (id, pubkey, kind, created_at, d_tag, raw_json, relay_url, seen_at)
         VALUES (?, ?, 8383, ?, NULL, '{}', 'wss://relay.mostro.network', ?)",
    )
    .bind("event-id")
    .bind("82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390")
    .bind(1_735_689_600_i64)
    .bind(1_735_689_600_i64)
    .execute(&pool)
    .await
    .expect("insert event");

    let result = sqlx::query(
        "INSERT INTO dev_fees
             (event_id, pubkey, order_id, amount_sats, payment_hash, destination, network,
              created_at, is_duplicate)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
    )
    .bind("event-id")
    .bind("82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390")
    .bind("an-order-nobody-has-seen")
    .bind(21_i64)
    .bind("payment-hash")
    .bind("mostro@getalby.com")
    .bind("mainnet")
    .bind(1_735_689_600_i64)
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "orphan dev fees must be storable: {result:?}"
    );
}

#[tokio::test]
async fn a_file_database_uses_write_ahead_logging() {
    // WAL is what lets a stats run read while sync writes. In-memory databases
    // ignore the setting, so this has to be checked on a real file.
    let dir = tempfile::tempdir().expect("temp dir");
    let url = format!("sqlite://{}", dir.path().join("wal.db").display());
    let pool = connect(&url).await.expect("connect");

    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .expect("pragma");

    assert_eq!(mode.to_lowercase(), "wal");
}

#[tokio::test]
async fn a_url_without_a_sqlite_scheme_is_rejected_rather_than_treated_as_a_filename() {
    // Found by this test: `SqliteConnectOptions` accepts a bare string as a
    // path, so `connect("not-a-sqlite-url")` used to succeed and leave a file
    // of that name in the working directory. A typo has to fail loudly.
    let error = connect("not-a-sqlite-url").await.expect_err("bad url");

    assert!(matches!(error, DbError::NotSqlite { .. }), "got {error:?}");
    assert!(error.to_string().contains("not-a-sqlite-url"));
    assert!(
        !std::path::Path::new("not-a-sqlite-url").exists(),
        "a rejected URL must not have created a database"
    );
}

#[tokio::test]
async fn an_unopenable_path_is_reported_rather_than_panicking() {
    let error = connect("sqlite:///nonexistent-directory/bestiario.db")
        .await
        .expect_err("unopenable path");

    assert!(
        matches!(error, DbError::Url { .. } | DbError::Connect { .. }),
        "got {error:?}"
    );
}

#[test]
fn in_memory_urls_get_a_single_pinned_connection() {
    // Several connections would mean several different empty databases, and
    // migrations would appear to vanish between calls.
    for url in ["sqlite::memory:", "sqlite://file:x?mode=memory"] {
        let options = pool_options_for(url);
        assert_eq!(options.get_max_connections(), 1, "{url}");

        // And the one connection must never be reaped: it *is* the database.
        assert_eq!(options.get_min_connections(), 1, "{url}");
        assert_eq!(options.get_idle_timeout(), None, "{url}");
        assert_eq!(options.get_max_lifetime(), None, "{url}");
    }
}

#[test]
fn file_urls_keep_the_ordinary_pool_defaults() {
    // A file-backed database outlives its connections, so reaping an idle one
    // costs nothing and expiry is worth keeping.
    let options = pool_options_for("sqlite://bestiario.db");

    assert!(options.get_max_connections() > 1);
    assert!(options.get_idle_timeout().is_some());
    assert!(options.get_max_lifetime().is_some());
}

#[tokio::test]
async fn reaping_an_idle_connection_destroys_an_in_memory_database() {
    // Demonstrates the failure that `pool_options_for` exists to prevent, by
    // building the pool sqlx's defaults would have given us: one connection,
    // none pinned, and an idle timeout. Shortened to milliseconds so the test
    // does not have to wait out the real 600 seconds.
    //
    // If this ever starts passing without an error, the hazard has gone away
    // and the pinning below can be reconsidered.
    let reaping_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .idle_timeout(Duration::from_millis(50))
        .connect_with(connect_options_for(MEMORY).expect("options"))
        .await
        .expect("connect");
    migrate(&reaping_pool).await.expect("migrate");

    tokio::time::sleep(Duration::from_millis(600)).await;

    let result = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM relays")
        .fetch_one(&reaping_pool)
        .await;

    // Asserted on the message, not merely on `is_err`, so the test cannot pass
    // because the query failed for some unrelated reason.
    let error = result.expect_err(
        "the reaped connection should have taken the schema with it; \
         if this now succeeds, sqlx changed and the pinning can be revisited",
    );
    assert!(
        error.to_string().contains("no such table"),
        "expected the schema to be gone, got: {error}"
    );
}

#[tokio::test]
async fn a_pinned_in_memory_database_outlives_an_idle_period() {
    // The same wait, against the pool `connect` actually builds.
    let pool = connect_and_migrate(MEMORY).await.expect("migrate");
    sqlx::query("INSERT INTO relays (url, source, first_seen_at) VALUES (?, ?, ?)")
        .bind("wss://relay.mostro.network")
        .bind("config")
        .bind(1_735_689_600_i64)
        .execute(&pool)
        .await
        .expect("insert");

    tokio::time::sleep(Duration::from_millis(600)).await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relays")
        .fetch_one(&pool)
        .await
        .expect("the schema should still exist");

    assert_eq!(count, 1);
    assert_eq!(table_names(&pool).await, EXPECTED_TABLES);
}
