//! The `indexed_kinds` table: which kinds bestiario has actually asked the
//! relays for, and how far back it asked.
//!
//! The archive answers "what was published"; this table answers "what was
//! looked for". They are different questions, and coverage needs both: a
//! kind with no rows in `events` is a confirmed zero only if something once
//! went and asked for it. `backfill --kind 38383` is a supported mode, so
//! the difference is not hypothetical.
//!
//! Only the floor is recorded, not a window: an indexer reads forward to
//! now by construction, and it is the reach *backwards* that varies between
//! a `--from`-bounded walk and one asking a relay for everything it holds.

use sqlx::{Executor, Sqlite};

/// Records that `kind` has been requested from `indexed_from` onwards.
///
/// The floor only deepens. A `--from 2026-08-01` backfill after a full one
/// does not narrow what the archive can speak for — the older events are
/// still stored — so the lowest floor ever asked for is the one kept.
pub async fn record<'e, E>(
    executor: E,
    kind: u16,
    indexed_from: i64,
    now: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO indexed_kinds (kind, indexed_from, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(kind) DO UPDATE SET
           indexed_from = MIN(indexed_kinds.indexed_from, excluded.indexed_from),
           updated_at   = MAX(indexed_kinds.updated_at, excluded.updated_at)",
    )
    .bind(i64::from(kind))
    .bind(indexed_from.max(0))
    .bind(now)
    .execute(executor)
    .await?;

    Ok(())
}

/// How far back `kind` has been asked for; `None` when it never has.
pub async fn indexed_from<'e, E>(executor: E, kind: u16) -> Result<Option<i64>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar("SELECT indexed_from FROM indexed_kinds WHERE kind = ?")
        .bind(i64::from(kind))
        .fetch_optional(executor)
        .await
}

#[cfg(test)]
mod tests;
