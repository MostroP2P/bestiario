//! The `events` table: every event bestiario has accepted, verbatim.
//!
//! This is the dedup gate of `docs/SPEC.md` §8.1 step 6 and the archive the
//! `rebuild --from-raw` command re-derives everything else from, so the row is
//! written before anything is parsed out of it and is never updated
//! afterwards: the first relay to deliver an event is the one recorded, and a
//! second copy from a second relay changes nothing.

use nostr_sdk::prelude::Event;
use sqlx::{Executor, QueryBuilder, Sqlite, SqlitePool};

use crate::db::load::Scope;

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

/// How far back the archive can speak for a report reading `kinds`, within
/// `scope`.
///
/// Three floors, and the latest of them wins.
///
/// The archive's own earliest event says when bestiario started indexing at
/// all. The earliest event *of those kinds* says when it started holding
/// what this report reads — which is later whenever a relay had already
/// expired them: orders live about a fortnight on a relay and dev fees
/// about a year, so a first backfill brings January's fees and only
/// August's orders. Taking the archive's floor alone would call January's
/// order-days covered and report zeros for a month the network was trading.
///
/// The third is for a kind the archive holds none of. Absent is ambiguous —
/// nobody published one, or nobody ever asked — and `backfill --kind 38383`
/// is a supported mode, so the difference is real. The answer comes from
/// [`indexed_kinds`](super::indexed_kinds), which records what was
/// requested and from when. A kind that was requested and came back empty
/// is a confirmed zero from its recorded floor; a kind nobody ever
/// requested is unknown history, and the report can speak for none of the
/// window rather than print observed zeros for it.
///
/// `scope` narrows both `events` floors to the instance the report covers,
/// so an instance added and backfilled after an older one's events expired
/// does not inherit the older one's reach. Only the instance half of the
/// scope applies: `events` stores each event verbatim and has no `network`
/// column, and a floor pretending otherwise would be the same lie in a
/// smaller place.
///
/// `None` when nothing can be spoken for: an empty archive, or a kind that
/// was never asked for.
pub async fn earliest_created_at(
    pool: &SqlitePool,
    kinds: &[u16],
    scope: &Scope,
) -> Result<Option<i64>, sqlx::Error> {
    let Some(archive) = scoped_min(pool, None, scope).await? else {
        return Ok(None);
    };

    let mut floor = archive;
    for &kind in kinds {
        let known = match scoped_min(pool, Some(kind), scope).await? {
            Some(first) => first,
            // Never stored. Only an explicit record of the request makes
            // that a zero rather than a blank.
            None => match super::indexed_kinds::indexed_from(pool, kind).await? {
                Some(from) => from,
                None => return Ok(None),
            },
        };
        floor = floor.max(known);
    }

    Ok(Some(floor))
}

/// The earliest `created_at` in `scope`, of `kind` or of every kind.
async fn scoped_min(
    pool: &SqlitePool,
    kind: Option<u16>,
    scope: &Scope,
) -> Result<Option<i64>, sqlx::Error> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT MIN(created_at) FROM events WHERE 1 = 1");
    if let Some(kind) = kind {
        query.push(" AND kind = ").push_bind(i64::from(kind));
    }
    scope.apply_instance(&mut query, "events");

    query.build_query_scalar().fetch_one(pool).await
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
