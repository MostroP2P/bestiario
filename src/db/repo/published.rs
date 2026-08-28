//! The `published_documents` and `publication_runs` tables: what has been
//! published, so a later run can say what changed
//! (`docs/NOSTR-PUBLICATION.md` §8).
//!
//! Responsibility: read the last publication's conclusions back as the
//! plain data `bestiario_stats::publish::restatement` compares against,
//! and write this run's conclusions in their place. No revision is decided
//! here — that is the rule, and the rule lives in the pure layer.
//!
//! # Why this is in the archive and not on a relay
//!
//! Reading the last index back off a relay would work until the day a
//! relay pruned it, and then every revision would silently reset to 1 —
//! which is precisely the claim §8 exists to make trustworthy. §9.3
//! refuses to keep a cache of *signed events*, state that can disagree
//! with the archive it came from; these tables hold no events and no
//! signatures, and every document is recomputed from the archive on every
//! run regardless.

use std::collections::BTreeMap;

use sqlx::{Executor, Row, Sqlite};

use crate::stats::publish::document::Restatement;
use crate::stats::publish::restatement::Previous;

#[cfg(test)]
mod tests;

/// One publication run, as the next one reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub snapshot_id: String,
    pub generated_at: i64,
    pub schema_version: u32,
    /// The coverage that run published, both ends; `None` when the
    /// archive held nothing.
    pub first_event_at: Option<i64>,
    pub last_event_at: Option<i64>,
    /// How many events the archive held. Coverage alone cannot see a
    /// backfill that found events inside a range already covered.
    pub events: u64,
}

/// What was last published, keyed by `d`.
///
/// A row whose `revision` is above the first but whose restatement is
/// missing cannot be represented by [`Previous`], and is read back as a
/// first publication rather than as a revision that restates nothing.
/// The database allows that shape; the type does not, and the type is the
/// one the rule of §8 is written against.
pub async fn all<'e, E>(executor: E) -> Result<BTreeMap<String, Previous>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rows = sqlx::query(
        "SELECT d, hash, revision, updated_at, restated_at, restated_because
         FROM published_documents",
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let hash: String = row.get("hash");
            let revision: i64 = row.get("revision");
            let updated_at: i64 = row.get("updated_at");
            let restatement = row
                .get::<Option<i64>, _>("restated_at")
                .zip(row.get::<Option<String>, _>("restated_because"))
                .map(|(at, because)| Restatement { at, because });

            let previous = restatement
                .and_then(|restatement| {
                    Previous::restated(
                        hash.clone(),
                        revision.max(0) as u32,
                        updated_at,
                        restatement,
                    )
                })
                .unwrap_or(Previous::First { hash, updated_at });

            (row.get::<String, _>("d"), previous)
        })
        .collect())
}

/// Records what one address was published as.
///
/// An upsert rather than a delete-and-insert of the whole set: an address
/// this run did not compute — a partition that fell outside coverage —
/// is still published, still on the relay under its own `d`, and
/// forgetting it here would restart its revision at 1 the next time it
/// came back.
pub async fn record<'e, E>(
    executor: E,
    address: &str,
    previous: &Previous,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let restatement = previous.restatement();

    sqlx::query(
        "INSERT INTO published_documents
           (d, hash, revision, updated_at, restated_at, restated_because)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(d) DO UPDATE SET
           hash             = excluded.hash,
           revision         = excluded.revision,
           updated_at       = excluded.updated_at,
           restated_at      = excluded.restated_at,
           restated_because = excluded.restated_because",
    )
    .bind(address)
    .bind(previous.hash())
    .bind(i64::from(previous.revision()))
    .bind(previous.updated_at())
    .bind(restatement.map(|r| r.at))
    .bind(restatement.map(|r| r.because.clone()))
    .execute(executor)
    .await?;

    Ok(())
}

/// Records that a run published, and what it read.
pub async fn record_run<'e, E>(executor: E, run: &Run) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO publication_runs
           (snapshot_id, generated_at, schema_version, first_event_at, last_event_at, events)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(snapshot_id) DO UPDATE SET
           generated_at   = excluded.generated_at,
           schema_version = excluded.schema_version,
           first_event_at = excluded.first_event_at,
           last_event_at  = excluded.last_event_at,
           events         = excluded.events",
    )
    .bind(&run.snapshot_id)
    .bind(run.generated_at)
    .bind(i64::from(run.schema_version))
    .bind(run.first_event_at)
    .bind(run.last_event_at)
    .bind(run.events as i64)
    .execute(executor)
    .await?;

    Ok(())
}

/// The most recent run, by its clock; `None` before anything was ever
/// published.
pub async fn latest_run<'e, E>(executor: E) -> Result<Option<Run>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT snapshot_id, generated_at, schema_version, first_event_at, last_event_at, events
         FROM publication_runs
         ORDER BY generated_at DESC, snapshot_id DESC
         LIMIT 1",
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(|row| Run {
        snapshot_id: row.get("snapshot_id"),
        generated_at: row.get("generated_at"),
        schema_version: row.get::<i64, _>("schema_version").max(0) as u32,
        first_event_at: row.get("first_event_at"),
        last_event_at: row.get("last_event_at"),
        events: row.get::<i64, _>("events").max(0) as u64,
    }))
}
