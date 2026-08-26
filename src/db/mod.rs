//! SQLite persistence: connection pool, migrations and repositories.
//!
//! Responsibility: all knowledge of the schema in `docs/SPEC.md` §4. Every
//! write is idempotent — the same event ingested twice must leave the
//! database in the same state as ingesting it once.

use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod repo;

#[cfg(test)]
mod tests;

/// Embedded at compile time, so a binary can migrate a database without the
/// `migrations/` directory being present next to it.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// How long to wait for a competing writer before giving up. SQLite allows one
/// writer at a time; `sync` writing while a `stats` run reads is the expected
/// case, and the default of zero would surface that as an error rather than a
/// short wait.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Anything that can go wrong opening or migrating the database.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("`{url}` is not a SQLite URL: expected it to start with `sqlite:`")]
    NotSqlite { url: String },

    #[error("`{url}` is not a usable SQLite URL")]
    Url {
        url: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("could not open the database at `{url}`")]
    Connect {
        url: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("could not apply migrations")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Opens the pool described by `url`, creating the database file if it does
/// not exist yet.
///
/// The connection is configured for a long-lived indexer that writes from one
/// process and reads from another:
///
/// - **WAL journal**, so a `stats` run reads while `sync` writes.
/// - **`foreign_keys = ON`**, which SQLite leaves off by default; the schema
///   declares references and they are worth enforcing.
/// - **`synchronous = NORMAL`**, the usual companion to WAL: durable against
///   process crashes, which is the failure this indexer can actually suffer.
///   Anything lost to a power cut is re-fetched from the relays.
/// - **a busy timeout**, so concurrent access waits instead of failing.
pub async fn connect(url: &str) -> Result<SqlitePool, DbError> {
    // `SqliteConnectOptions` accepts a bare string as a filename, so without
    // this a typo does not fail — it quietly creates a database named after
    // the typo and reports success. Configuration already rejects non-SQLite
    // URLs, but this function is public and has to be safe on its own.
    if !url.starts_with("sqlite:") {
        return Err(DbError::NotSqlite {
            url: url.to_string(),
        });
    }

    pool_options_for(url)
        .connect_with(connect_options_for(url)?)
        .await
        .map_err(|source| DbError::Connect {
            url: url.to_string(),
            source,
        })
}

/// Applies every pending migration. Safe to call on an already-migrated
/// database: `sqlx` records what it has run and applies only the rest.
pub async fn migrate(pool: &SqlitePool) -> Result<(), DbError> {
    MIGRATOR.run(pool).await?;
    Ok(())
}

/// Opens a pool and brings it up to date, which is what every command wants.
pub async fn connect_and_migrate(url: &str) -> Result<SqlitePool, DbError> {
    let pool = connect(url).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// The per-connection settings. Split out so that tests can build a pool that
/// differs only in its pool policy.
fn connect_options_for(url: &str) -> Result<SqliteConnectOptions, DbError> {
    Ok(SqliteConnectOptions::from_str(url)
        .map_err(|source| DbError::Url {
            url: url.to_string(),
            source,
        })?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT))
}

/// An in-memory database lives *inside* the connection that opened it. Two
/// consequences, both of which the defaults get wrong:
///
/// - A pool of several connections is several *different* empty databases, so
///   migrations appear to vanish between calls. Hence one connection.
/// - `sqlx` reaps connections that sit idle (600s by default) or grow old
///   (1800s), and with `min_connections` at zero it may reap the only one
///   there is. That does not just drop a connection, it destroys the
///   database: the next acquire opens a fresh, empty, unmigrated one. So the
///   connection is pinned and expiry is switched off.
///
/// File-backed databases have neither problem — the data outlives any
/// connection — so they keep the ordinary defaults.
fn pool_options_for(url: &str) -> SqlitePoolOptions {
    let options = SqlitePoolOptions::new();

    if is_in_memory(url) {
        options
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
    } else {
        options.max_connections(5)
    }
}

fn is_in_memory(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}
