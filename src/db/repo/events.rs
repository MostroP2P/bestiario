//! The `events` table: every event bestiario has accepted, verbatim.
//!
//! This is the dedup gate of `docs/SPEC.md` §8.1 step 6 and the archive the
//! `rebuild --from-raw` command re-derives everything else from, so the row is
//! written before anything is parsed out of it and is never updated
//! afterwards: the first relay to deliver an event is the one recorded, and a
//! second copy from a second relay changes nothing.

use nostr_sdk::prelude::Event;
use sqlx::{Executor, Sqlite};

/// One row of `events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub id: String,
    pub pubkey: String,
    pub kind: i64,
    /// The event's own `created_at`, not the moment it was received.
    pub created_at: i64,
    /// The `d` tag of an addressable event; `None` for the regular kinds.
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

#[cfg(test)]
mod tests;
