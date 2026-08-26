//! Every published version of a dispute, and the projection derived from them.
//!
//! The same shape as [`super::orders`], for the same reason: `dispute_versions`
//! is append-only and authoritative, and the `disputes` row holds nothing that
//! cannot be recomputed from it (`docs/SPEC.md` §4). Backfill walks backwards,
//! so a `settled` version routinely arrives before the `initiated` one, and
//! recomputing from the whole history is what makes arrival order stop
//! mattering.
//!
//! There is no order reference anywhere here, because kind 38386 publishes
//! none (SPEC §2.3). What can still be measured is the aggregate rate of
//! SPEC §6.7 — disputes opened against orders that left `pending` — which
//! divides two counts and never pairs a dispute with its order.

use sqlx::{Executor, Sqlite};

use crate::ingest::parse::dispute::{DisputeVersion, Initiator, Status};

use super::decode;

/// One row of `disputes`: a dispute as it currently stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispute {
    pub dispute_id: String,
    pub pubkey: String,
    /// When the dispute was opened — from the `created_at` *tag*, not the
    /// event's own clock. `None` while no version has published one.
    pub opened_at: Option<i64>,
    /// `created_at` of the latest version seen.
    pub last_updated_at: i64,
    pub final_status: Status,
    pub initiator: Option<Initiator>,
}

/// Stores one published version, ignoring a version already known.
pub async fn insert_version<'e, E>(executor: E, version: &DisputeVersion) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT OR IGNORE INTO dispute_versions (
             event_id, dispute_id, pubkey, created_at, status, initiator, opened_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&version.event_id)
    .bind(&version.dispute_id)
    .bind(&version.pubkey)
    .bind(version.created_at)
    .bind(version.status.as_str())
    .bind(version.initiator.map(Initiator::as_str))
    .bind(version.opened_at)
    .execute(executor)
    .await?;

    Ok(())
}

/// Recomputes the `disputes` row for `dispute_id` from every version stored.
///
/// The status comes from the latest version. `opened_at` and `initiator` come
/// from the most recent version that actually published one: a later version
/// that omits the tag is saying nothing about it, not that the dispute has
/// stopped having an initiator, and blanking the projection on such a version
/// would lose a fact the history still holds.
///
/// Two versions can share a `created_at`, since it has one-second resolution.
/// NIP-01 settles that tie for an addressable event by retaining the
/// lexicographically **lowest** event id, so that is the version projected
/// here: picking the other one would make the dispute counts disagree with
/// what the relays themselves keep.
///
/// A dispute with no versions leaves no row, so that a rebuild that empties
/// the version table does not resurrect it.
pub async fn refresh_projection<'e, E>(executor: E, dispute_id: &str) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO disputes (
             dispute_id, pubkey, opened_at, last_updated_at, final_status, initiator
         )
         SELECT latest.dispute_id, latest.pubkey, known.opened_at, known.last_updated_at,
                latest.status, known.initiator
         FROM (
             SELECT * FROM dispute_versions WHERE dispute_id = ?1
             ORDER BY created_at DESC, event_id ASC LIMIT 1
         ) AS latest
         JOIN (
             SELECT MAX(created_at) AS last_updated_at,
                    (SELECT opened_at FROM dispute_versions
                     WHERE dispute_id = ?1 AND opened_at IS NOT NULL
                     ORDER BY created_at DESC, event_id ASC LIMIT 1) AS opened_at,
                    (SELECT initiator FROM dispute_versions
                     WHERE dispute_id = ?1 AND initiator IS NOT NULL
                     ORDER BY created_at DESC, event_id ASC LIMIT 1) AS initiator
             FROM dispute_versions WHERE dispute_id = ?1
         ) AS known
         -- Required, not decoration: without it SQLite reads the following ON
         -- as another join condition rather than as the upsert clause.
         WHERE true
         ON CONFLICT(dispute_id) DO UPDATE SET
             pubkey = excluded.pubkey,
             opened_at = excluded.opened_at,
             last_updated_at = excluded.last_updated_at,
             final_status = excluded.final_status,
             initiator = excluded.initiator",
    )
    .bind(dispute_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// The projected dispute, if any version of it has been seen.
pub async fn find<'e, E>(executor: E, dispute_id: &str) -> Result<Option<Dispute>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query_as::<_, DisputeRow>(
        "SELECT dispute_id, pubkey, opened_at, last_updated_at, final_status, initiator
         FROM disputes WHERE dispute_id = ?",
    )
    .bind(dispute_id)
    .fetch_optional(executor)
    .await?;

    row.map(DisputeRow::into_dispute).transpose()
}

/// Every stored version of `dispute_id`, oldest first.
pub async fn versions<'e, E>(
    executor: E,
    dispute_id: &str,
) -> Result<Vec<DisputeVersion>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, VersionRow>(
        "SELECT event_id, dispute_id, pubkey, created_at, status, initiator, opened_at
         FROM dispute_versions WHERE dispute_id = ? ORDER BY created_at, event_id",
    )
    .bind(dispute_id)
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(VersionRow::into_version)
    .collect()
}

/// The stored `initiator`, if the column is not NULL.
fn optional_initiator(stored: Option<&str>) -> Result<Option<Initiator>, sqlx::Error> {
    match stored {
        None => Ok(None),
        Some(wire) => decode("initiator", Initiator::parse(wire)).map(Some),
    }
}

/// The `disputes` row exactly as SQLite holds it.
#[derive(sqlx::FromRow)]
struct DisputeRow {
    dispute_id: String,
    pubkey: String,
    opened_at: Option<i64>,
    last_updated_at: i64,
    final_status: String,
    initiator: Option<String>,
}

impl DisputeRow {
    fn into_dispute(self) -> Result<Dispute, sqlx::Error> {
        Ok(Dispute {
            dispute_id: self.dispute_id,
            pubkey: self.pubkey,
            opened_at: self.opened_at,
            last_updated_at: self.last_updated_at,
            final_status: decode("final_status", Status::parse(&self.final_status))?,
            initiator: optional_initiator(self.initiator.as_deref())?,
        })
    }
}

/// The `dispute_versions` row exactly as SQLite holds it.
#[derive(sqlx::FromRow)]
struct VersionRow {
    event_id: String,
    dispute_id: String,
    pubkey: String,
    created_at: i64,
    status: String,
    initiator: Option<String>,
    opened_at: Option<i64>,
}

impl VersionRow {
    fn into_version(self) -> Result<DisputeVersion, sqlx::Error> {
        Ok(DisputeVersion {
            event_id: self.event_id,
            dispute_id: self.dispute_id,
            pubkey: self.pubkey,
            created_at: self.created_at,
            status: decode("status", Status::parse(&self.status))?,
            initiator: optional_initiator(self.initiator.as_deref())?,
            opened_at: self.opened_at,
        })
    }
}

/// Every dispute any version has been seen of.
pub async fn ids<'e, E>(executor: E) -> Result<Vec<String>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT dispute_id FROM dispute_versions ORDER BY dispute_id",
    )
    .fetch_all(executor)
    .await
}

/// Empties the projection, leaving the versions it is derived from.
pub async fn clear_projection<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM disputes")
        .execute(executor)
        .await?;
    Ok(())
}

/// Empties the version table; see [`super::orders::clear_versions`].
pub async fn clear_versions<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM dispute_versions")
        .execute(executor)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
