//! The `events` table: every event bestiario has accepted, verbatim.
//!
//! This is the dedup gate of `docs/SPEC.md` §8.1 step 6 and the archive the
//! `rebuild --from-raw` command re-derives everything else from, so the row is
//! written before anything is parsed out of it and is never updated
//! afterwards: the first relay to deliver an event is the one recorded, and a
//! second copy from a second relay changes nothing.

use nostr_sdk::prelude::Event;
use sqlx::{Executor, QueryBuilder, Sqlite};

/// One row of `events`.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct EventRecord {
    pub id: String,
    pub pubkey: String,
    pub kind: i64,
    /// The event's own `created_at`, not the moment it was received.
    pub created_at: i64,
    /// The `d` tag of an addressable event; `None` for the regular kinds, and
    /// `None` too when the event repeats the tag and so has no single key.
    pub d_tag: Option<String>,
    pub raw_json: String,
    pub relay_url: String,
    /// When bestiario saw it — wall clock, and the only field here that is.
    pub seen_at: i64,
}

impl EventRecord {
    /// The row for `event`, as delivered by `relay_url` at `seen_at`.
    pub fn new(event: &Event, relay_url: &str, seen_at: i64) -> Self {
        Self {
            id: event.id.to_hex(),
            pubkey: event.pubkey.to_hex(),
            kind: i64::from(event.kind.as_u16()),
            created_at: event.created_at.as_secs() as i64,
            d_tag: crate::ingest::parse::tag_values(event, "d")
                .ok()
                .flatten()
                .and_then(|values| values.first().cloned()),
            raw_json: event.as_json(),
            relay_url: relay_url.to_string(),
            seen_at,
        }
    }
}

/// Stores `record` unless its id is already known.
///
/// Returns whether the row was new. That boolean is the dedup gate: the
/// pipeline stops on `false`, which is what makes re-ingesting a relay's
/// backlog cheap and what keeps an event delivered by three relays from being
/// counted three times.
pub async fn insert_if_new<'e, E>(executor: E, record: &EventRecord) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(
        "INSERT OR IGNORE INTO events
           (id, pubkey, kind, created_at, d_tag, raw_json, relay_url, seen_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&record.id)
    .bind(&record.pubkey)
    .bind(record.kind)
    .bind(record.created_at)
    .bind(&record.d_tag)
    .bind(&record.raw_json)
    .bind(&record.relay_url)
    .bind(record.seen_at)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Whether an event id is already stored, for callers that want to know
/// without writing anything.
pub async fn exists<'e, E>(executor: E, id: &str) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let found: Option<String> = sqlx::query_scalar("SELECT id FROM events WHERE id = ?")
        .bind(id)
        .fetch_optional(executor)
        .await?;

    Ok(found.is_some())
}

/// Whether nothing has been stored yet — a database that was migrated
/// and never backfilled.
pub async fn is_empty<'e, E>(executor: E) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(executor)
        .await?;
    Ok(count == 0)
}

/// How far back the archive can speak for a report reading `kinds`.
///
/// Two floors, and the later of them wins. The archive's own earliest
/// event says when bestiario started indexing at all. The earliest event
/// *of those kinds* says when it started holding what this report reads —
/// which is later whenever a relay had already expired them: orders live
/// about a fortnight on a relay and dev fees about a year, so a first
/// backfill brings January's fees and only August's orders. Taking the
/// archive's floor alone would call January's order-days covered and
/// report zeros for a month the network was trading.
///
/// When the archive holds none of `kinds` at all, its own floor is the
/// answer: bestiario was indexing, and there were none to see.
/// `None` only when the archive is empty.
pub async fn earliest_created_at<'e, E>(
    executor: E,
    kinds: &[u16],
) -> Result<Option<i64>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    // The latest of the per-kind floors, not the earliest across them: a
    // report reading two kinds can only speak for a period it holds both
    // of, and `MIN` over the pair would take the kind the relays kept
    // longest and call the other's gap covered.
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT (SELECT MIN(created_at) FROM events) AS archive,
                (SELECT MAX(first) FROM (
                     SELECT MIN(created_at) AS first FROM events WHERE kind IN (",
    );
    let mut list = query.separated(", ");
    for kind in kinds {
        list.push_bind(i64::from(*kind));
    }
    query.push(" ) GROUP BY kind)) AS family");

    let (archive, family): (Option<i64>, Option<i64>) =
        query.build_query_as().fetch_one(executor).await?;

    Ok(match (archive, family) {
        (Some(archive), Some(family)) => Some(archive.max(family)),
        (archive, None) => archive,
        (None, family) => family,
    })
}

/// Every archived event, oldest first.
///
/// Ordered so that a replay applies versions in the order the network
/// published them: the projections are order-independent by construction, but
/// a deterministic replay is what makes `rebuild` comparable to the run it is
/// rebuilding.
pub async fn all<'e, E>(executor: E) -> Result<Vec<EventRecord>, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query_as::<_, EventRecord>(
        "SELECT id, pubkey, kind, created_at, d_tag, raw_json, relay_url, seen_at
         FROM events ORDER BY created_at, id",
    )
    .fetch_all(executor)
    .await
}

#[cfg(test)]
mod tests;
