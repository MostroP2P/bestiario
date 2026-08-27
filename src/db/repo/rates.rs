//! Exchange rate snapshots, one row per kind 30078 event.
//!
//! Every snapshot is kept, never the latest alone: valuing an order means
//! finding the rate in force when it completed (`docs/SPEC.md` §5), and a
//! table holding only today's rate could only value today's orders.
//!
//! The rates travel as the JSON object the instance published, keyed by
//! currency code. One row per snapshot rather than one per currency: a
//! snapshot is read whole — the lookup wants the newest snapshot at or
//! before a moment and then one code out of it — and a hundred-odd rows per
//! hourly event would be a table nobody queries by row.

use std::collections::BTreeMap;

use sqlx::{Executor, Sqlite};

use crate::ingest::parse::rates::RateSnapshot;

/// Stores `snapshot`, ignoring one already known.
pub async fn insert<'e, E>(executor: E, snapshot: &RateSnapshot) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let rates_json = serde_json::to_string(&snapshot.rates)
        .expect("a map of strings to finite floats serialises");

    sqlx::query(
        "INSERT OR IGNORE INTO rates (event_id, pubkey, published_at, source, rates_json)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&snapshot.event_id)
    .bind(&snapshot.pubkey)
    .bind(snapshot.published_at)
    .bind(&snapshot.source)
    .bind(rates_json)
    .execute(executor)
    .await?;

    Ok(())
}

/// Every snapshot stored, oldest first — what the rate lookup of
/// `bestiario::stats` is handed.
pub async fn all<'e, E>(executor: E) -> Result<Vec<RateSnapshot>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Row>(
        "SELECT event_id, pubkey, published_at, source, rates_json
         FROM rates ORDER BY published_at, event_id",
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Row::into_snapshot)
    .collect()
}

/// The snapshots published inside `[from, until)`, oldest first.
///
/// The lookup only ever consults a snapshot published at or before the
/// instant asked about and no more than `MAX_AGE_SECS` earlier, so a report
/// over a window needs no snapshot older than its floor minus that bound —
/// and none at all from after it. Loading the lifetime table to answer for
/// one week grows with the archive rather than with the question.
pub async fn published_between<'e, E>(
    executor: E,
    from: i64,
    until: i64,
) -> Result<Vec<RateSnapshot>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Row>(
        "SELECT event_id, pubkey, published_at, source, rates_json
         FROM rates WHERE published_at >= ? AND published_at < ?
         ORDER BY published_at, event_id",
    )
    .bind(from)
    .bind(until)
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Row::into_snapshot)
    .collect()
}

/// The latest snapshot of every instance that has published one, by
/// pubkey — what the §6.8 report asks about, which is the present state of
/// each feed and not its history. `pubkey` narrows it to one publisher.
///
/// # Which snapshot is the latest
///
/// Kind 30078 is addressable, and NIP-01 settles which of two versions is
/// the current one by `created_at`, the id breaking a tie — not by the
/// `published_at` tag, which is the instance's own claim and may sit up to
/// `MAX_CLOCK_DIVERGENCE_SECS` either side of it. Two snapshots whose
/// tags and clocks disagree in order would otherwise report the rate and
/// the freshness of an event the relay has already replaced, so the rank
/// joins `events` and orders the way the protocol does.
///
/// The scope is pushed into the query rather than applied after it: a
/// report for one instance should not rank every publisher's history, and
/// should not decode — nor be failed by — a corrupt row belonging to
/// somebody else.
pub async fn latest_per_instance<'e, E>(
    executor: E,
    pubkey: Option<&str>,
) -> Result<Vec<RateSnapshot>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, Row>(
        "SELECT event_id, pubkey, published_at, source, rates_json
         FROM (
             SELECT rates.event_id, rates.pubkey, rates.published_at,
                    rates.source, rates.rates_json,
                    ROW_NUMBER() OVER (
                        PARTITION BY rates.pubkey
                        ORDER BY events.created_at DESC, rates.event_id ASC
                    ) AS rank
             FROM rates JOIN events ON events.id = rates.event_id
             WHERE ?1 IS NULL OR rates.pubkey = ?1
         )
         WHERE rank = 1
         ORDER BY pubkey",
    )
    .bind(pubkey)
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Row::into_snapshot)
    .collect()
}

/// Empties the table; see [`super::orders::clear_versions`].
pub async fn clear<'e, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("DELETE FROM rates").execute(executor).await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Row {
    event_id: String,
    pubkey: String,
    published_at: i64,
    source: Option<String>,
    rates_json: String,
}

impl Row {
    fn into_snapshot(self) -> Result<RateSnapshot, sqlx::Error> {
        let rates: BTreeMap<String, f64> =
            serde_json::from_str(&self.rates_json).map_err(|error| sqlx::Error::ColumnDecode {
                index: "rates_json".to_string(),
                source: error.into(),
            })?;

        Ok(RateSnapshot {
            event_id: self.event_id,
            pubkey: self.pubkey,
            published_at: self.published_at,
            source: self.source,
            rates,
        })
    }
}

#[cfg(test)]
mod tests;
