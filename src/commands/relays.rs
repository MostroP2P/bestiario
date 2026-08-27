//! The relays a run dials: the configured ones, and — when
//! `discover_relays` is on — those the instances advertise (SPEC §2.6).
//!
//! Discovery is opt-in and additive. The configured relays always come
//! first and are never dropped: they are the operator's decision, and a
//! discovered relay is a claim by a third party about where it publishes.
//! With the flag off the set is exactly what the operator wrote, whatever
//! the instances have advertised and whatever is already in the table.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::config::Settings;
use crate::db::repo::relays::{self, Source};

/// The relay URLs to connect to, configured first.
///
/// Records the configured relays as `config` on the way through, so the
/// table answers where every relay bestiario knows came from.
pub async fn connection_set(
    pool: &SqlitePool,
    settings: &Settings,
    now: i64,
) -> Result<Vec<String>> {
    let mut set = settings.nostr.relays.clone();
    for url in &set {
        relays::upsert(pool, url, &Source::Config, now).await?;
    }

    if settings.nostr.discover_relays {
        for relay in relays::discovered(pool).await? {
            if !set.contains(&relay.url) {
                set.push(relay.url);
            }
        }
    }

    Ok(set)
}

#[cfg(test)]
mod tests;
