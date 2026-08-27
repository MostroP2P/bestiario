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
