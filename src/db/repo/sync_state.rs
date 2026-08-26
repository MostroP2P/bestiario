//! Where each relay was left off, per kind.
//!
//! One cursor per `(relay_url, kind)` rather than one per relay: relays carry
//! different kinds to different depths, and a single cursor would let a relay
//! that is current on orders skip the dev fees it has never sent
//! (`docs/SPEC.md` §8.2).
//!
//! # Two clocks, deliberately
//!
//! `last_created_at` is the event's clock — it is what the next `since` filter
//! is built from, so it has to be comparable with what the relays publish.
//! `updated_at` is ours, and is only ever read by a human wondering when a
//! relay was last reached; nothing resumes from it.

use sqlx::{Executor, Sqlite};

/// One cursor: how far this relay has been read for this kind.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Cursor {
    pub relay_url: String,
    pub kind: i64,
    /// `created_at` of the newest event accepted from this relay and kind.
    pub last_created_at: i64,
    /// When bestiario last advanced this cursor — wall clock.
    pub updated_at: i64,
}

/// Moves the cursor to `created_at`, if that is further along than where it is.
///
/// Never backwards, in either clock. Backfill walks into the past and would
/// otherwise reset a cursor that live sync had already carried forward, making
/// the next `sync` re-read everything in between; and a relay replaying an old
/// event must not undo progress either.
///
/// `updated_at` takes the later of the two for the same reason. Backfill and
/// sync can be writing at once — the pool is configured for exactly that — so
/// a call that captured an earlier `now` may commit after a later one, and an
/// unconditional assignment would report a relay as last reached before it
/// actually was.
pub async fn advance<'e, E>(
    executor: E,
    relay_url: &str,
    kind: u16,
    created_at: i64,
    now: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO sync_state (relay_url, kind, last_created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(relay_url, kind) DO UPDATE SET
           last_created_at = MAX(sync_state.last_created_at, excluded.last_created_at),
           updated_at = MAX(sync_state.updated_at, excluded.updated_at)",
    )
    .bind(relay_url)
    .bind(i64::from(kind))
    .bind(created_at)
    .bind(now)
    .execute(executor)
    .await?;

    Ok(())
}

/// The cursor for `(relay_url, kind)`, or `None` if it has never been read.
///
/// `None` is what starts a backfill: there is no floor to resume from, so the
/// caller walks back to its configured `backfill_from` instead.
pub async fn get<'e, E>(
    executor: E,
    relay_url: &str,
    kind: u16,
) -> Result<Option<Cursor>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Cursor>(
        "SELECT relay_url, kind, last_created_at, updated_at
         FROM sync_state WHERE relay_url = ? AND kind = ?",
    )
    .bind(relay_url)
    .bind(i64::from(kind))
    .fetch_optional(executor)
    .await
}

/// Every cursor, by relay and then kind — what `sync` reports on start-up.
pub async fn all<'e, E>(executor: E) -> Result<Vec<Cursor>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Cursor>(
        "SELECT relay_url, kind, last_created_at, updated_at
         FROM sync_state ORDER BY relay_url, kind",
    )
    .fetch_all(executor)
    .await
}

#[cfg(test)]
mod tests;
