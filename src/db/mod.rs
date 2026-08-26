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

    #[error("`{url}` is not a usable SQLite URL: {source}")]
    Url {
        url: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("could not open the database at `{url}`: {source}")]
    Connect {
        url: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("could not apply migrations: {0}")]
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

    let options = SqliteConnectOptions::from_str(url)
        .map_err(|source| DbError::Url {
            url: url.to_string(),
            source,
        })?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT);

    SqlitePoolOptions::new()
        .max_connections(max_connections_for(url))
        .connect_with(options)
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

/// An in-memory database belongs to the connection that opened it, so a pool
/// of several would hand out connections to several *different* empty
/// databases — migrations would appear to vanish between calls. Tests are the
/// only user of these URLs, and one connection is all they need.
fn max_connections_for(url: &str) -> u32 {
    if is_in_memory(url) { 1 } else { 5 }
}

fn is_in_memory(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}
