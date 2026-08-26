//! The `instances` projection and the `instance_names` history.
//!
//! An instance is identified by its pubkey; its name is whatever it last
//! published in the second value of `y`, on any kind (`docs/SPEC.md` §3).
//!
//! # Why "most recent" means the event's clock, not ours
//!
//! Backfill walks *backwards*, so events arrive newest-first during one run
//! and oldest-first during another. Ordering names by when bestiario saw them
//! would let a backfill overwrite a current name with a year-old one. Every
//! comparison here is therefore against the event's own `created_at`, which is
//! the same order in either direction.

use sqlx::{Acquire, Executor, Sqlite};

/// One row of `instances`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Instance {
    pub pubkey: String,
    /// `None` for the third of the network that publishes no name.
    pub name: Option<String>,
    /// The `created_at` of the event [`name`](Self::name) came from.
    pub name_seen_at: Option<i64>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// Records that `pubkey` published an event at `published_at`, optionally
/// carrying `name`.
///
/// Idempotent and order-independent: replaying the same events in any order
/// leaves the same row. A name only wins if no newer name has been seen, and
/// an event with no name never clears one.
pub async fn upsert<'e, E>(
    executor: E,
    pubkey: &str,
    name: Option<&str>,
    published_at: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO instances (pubkey, name, name_seen_at, first_seen_at, last_seen_at)
         VALUES (?1, ?2, CASE WHEN ?2 IS NULL THEN NULL ELSE ?3 END, ?3, ?3)
         ON CONFLICT(pubkey) DO UPDATE SET
           first_seen_at = MIN(instances.first_seen_at, excluded.first_seen_at),
           last_seen_at  = MAX(instances.last_seen_at, excluded.last_seen_at),
           name = CASE
             WHEN excluded.name IS NULL THEN instances.name
             WHEN instances.name_seen_at IS NULL THEN excluded.name
             WHEN excluded.name_seen_at >= instances.name_seen_at THEN excluded.name
             ELSE instances.name
           END,
           name_seen_at = CASE
             WHEN excluded.name IS NULL THEN instances.name_seen_at
             WHEN instances.name_seen_at IS NULL THEN excluded.name_seen_at
             WHEN excluded.name_seen_at >= instances.name_seen_at THEN excluded.name_seen_at
             ELSE instances.name_seen_at
           END",
    )
    .bind(pubkey)
    .bind(name)
    .bind(published_at)
    .execute(executor)
    .await?;

    Ok(())
}

/// Adds `name` to the history of `pubkey`, keeping the most recent sighting of
/// that particular name.
///
/// Separate from [`upsert`] rather than folded into it because they write
/// different tables and the caller is already inside a transaction; keeping
/// them apart is what lets the pipeline order its writes.
pub async fn record_name<'e, E>(
    executor: E,
    pubkey: &str,
    name: &str,
    published_at: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO instance_names (pubkey, name, seen_at)
         VALUES (?, ?, ?)
         ON CONFLICT(pubkey, name) DO UPDATE SET
           seen_at = MAX(instance_names.seen_at, excluded.seen_at)",
    )
    .bind(pubkey)
    .bind(name)
    .bind(published_at)
    .execute(executor)
    .await?;

    Ok(())
}

/// The instance with this pubkey, if it has ever been seen.
pub async fn find<'e, E>(executor: E, pubkey: &str) -> Result<Option<Instance>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Instance>(
        "SELECT pubkey, name, name_seen_at, first_seen_at, last_seen_at
         FROM instances WHERE pubkey = ?",
    )
    .bind(pubkey)
    .fetch_optional(executor)
    .await
}

/// Every instance seen, oldest first — the bestiary itself.
pub async fn all<'e, E>(executor: E) -> Result<Vec<Instance>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Instance>(
        "SELECT pubkey, name, name_seen_at, first_seen_at, last_seen_at
         FROM instances ORDER BY first_seen_at, pubkey",
    )
    .fetch_all(executor)
    .await
}

/// Every name `pubkey` has published, most recent sighting first.
pub async fn names<'e, E>(executor: E, pubkey: &str) -> Result<Vec<(String, i64)>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, (String, i64)>(
        "SELECT name, seen_at FROM instance_names WHERE pubkey = ? ORDER BY seen_at DESC, name",
    )
    .bind(pubkey)
    .fetch_all(executor)
    .await
}

/// Empties both the bestiary and the name history.
///
/// The two go together: a name history without its instance is a set of rows
/// nothing points at, and the rebuild that refills them reads both from the
/// same pass over `events`.
pub async fn clear<'a, A>(acquirer: A) -> Result<(), sqlx::Error>
where
    A: Acquire<'a, Database = Sqlite>,
{
    let mut connection = acquirer.acquire().await?;

    sqlx::query("DELETE FROM instance_names")
        .execute(&mut *connection)
        .await?;
    sqlx::query("DELETE FROM instances")
        .execute(&mut *connection)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests;
