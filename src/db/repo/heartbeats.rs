//! Step 6a of `docs/SPEC.md` §8.1: whether a heartbeat republication says
//! anything the archive does not already hold.
//!
//! mostrod republishes its relay list (10002) about once a minute and its
//! info (38385) and rates (30078) about every five minutes, almost always
//! unchanged. Archiving every copy grew one month of production data past a
//! gigabyte — more than the orders the index exists for, by two orders of
//! magnitude. So:
//!
//! * a 10002 or 38385 is redundant when the version archived immediately
//!   before it, for the same publisher and address, carries the same content
//!   and the same tags, in any order. Compared against the *preceding* version rather than
//!   the latest one, so a backfill walking backwards still stores the first
//!   of a run of identical copies;
//! * a 30078 is redundant when a snapshot from the same publisher is already
//!   archived in the same hour of `published_at`. An hour is far finer than
//!   anything the rate lookup values an order at.
//!
//! Reads `events` and `rates` both, which is why it is a module of its own
//! rather than a query on either.

use nostr_sdk::prelude::Event;
use sqlx::{Executor, Sqlite};

/// How finely rate snapshots are kept, in seconds.
pub const RATES_BUCKET_SECS: i64 = 3600;

/// What the redundancy rule needs from a parsed event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// A replaceable announcement: 10002 or 38385.
    Announcement,
    /// A rate snapshot, keyed by the moment its rates were fetched.
    Rates { published_at: i64 },
    /// Every other kind, which is always worth keeping.
    None,
}

/// Whether `event` repeats what the archive already holds.
///
/// Never true of `event` itself: a second copy of an archived event is a
/// duplicate, which step 6 reports as such.
pub async fn is_redundant<'e, E>(
    executor: E,
    event: &Event,
    heartbeat: &Heartbeat,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id = event.id.to_hex();
    let pubkey = event.pubkey.to_hex();

    match heartbeat {
        Heartbeat::None => Ok(false),
        Heartbeat::Rates { published_at } => {
            let bucket = published_at - published_at.rem_euclid(RATES_BUCKET_SECS);
            sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS (
                     SELECT 1 FROM rates
                     WHERE pubkey = ? AND published_at >= ? AND published_at < ?
                       AND event_id != ?
                 )",
            )
            .bind(&pubkey)
            .bind(bucket)
            .bind(bucket + RATES_BUCKET_SECS)
            .bind(&id)
            .fetch_one(executor)
            .await
            .map(|exists| exists != 0)
        }
        Heartbeat::Announcement => {
            let d_tag = crate::ingest::parse::tag_values(event, "d")
                .ok()
                .flatten()
                .and_then(|values| values.first().cloned());
            let previous = sqlx::query_scalar::<_, String>(
                "SELECT raw_json FROM events
                 WHERE pubkey = ? AND kind = ? AND d_tag IS ? AND created_at <= ?
                   AND id != ?
                 ORDER BY created_at DESC, id ASC LIMIT 1",
            )
            .bind(&pubkey)
            .bind(i64::from(event.kind.as_u16()))
            .bind(d_tag)
            .bind(event.created_at.as_secs() as i64)
            .bind(&id)
            .fetch_optional(executor)
            .await?;

            Ok(previous
                .and_then(|raw| Event::from_json(raw).ok())
                .is_some_and(|previous| {
                    previous.content == event.content && same_tags(&previous, event)
                }))
        }
    }
}

/// Whether two events carry the same tags, in any order.
///
/// mostrod keeps its relays in a hash set, so each republication of an
/// unchanged relay list names them in a different order.
fn same_tags(a: &Event, b: &Event) -> bool {
    let sorted = |event: &Event| {
        let mut tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        tags.sort();
        tags
    };
    sorted(a) == sorted(b)
}
