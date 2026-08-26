//! `bestiario rebuild`: re-derive everything that was derived.
//!
//! The archive is `events.raw_json`; every other table is a function of it.
//! This command recomputes those functions, which is what makes a change to a
//! projection rule a code change rather than a migration.
//!
//! # What is rebuilt, and from what
//!
//! - `instances` and `instance_names` — from a pass over the archive, because
//!   the `y` tag they read lives in the raw event and nowhere else. SPEC §8.1
//!   writes them in the same step as the version tables, so a rebuild that
//!   skipped them would leave the bestiary stale while everything around it
//!   was current.
//! - `orders` and `disputes` — from `order_versions` and `dispute_versions`,
//!   by the same projection queries the pipeline uses.
//! - `dev_fees.is_duplicate` — recomputed per order, for the same reason:
//!   it is derived state, not published state.
//!
//! With `--from-raw` the version tables are emptied first, so the replay
//! refills them from the raw events too. Without it they are left alone and
//! the replay's inserts are no-ops, which is what makes a plain `rebuild`
//! cheap and a `--from-raw` one authoritative.
//!
//! # Why the admission rules are not re-applied
//!
//! An archived event was admitted once. Running it past today's
//! configuration again — a narrowed `networks` list, a shorter instance
//! allow-list — would delete history rather than rebuild it. See
//! [`Pipeline::replay`].

use anyhow::{Context as _, Result};
use nostr_sdk::prelude::Event;
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::db::repo;
use crate::ingest::{IngestOutcome, Pipeline, Policy};

/// What a rebuild did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rebuilt {
    /// Archived events replayed.
    pub events: u64,
    /// Archived events no parser could read. They stay in the archive.
    pub unreadable: u64,
    pub orders: u64,
    pub disputes: u64,
}

impl std::fmt::Display for Rebuilt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} events replayed ({} unreadable), {} orders and {} disputes projected",
            self.events, self.unreadable, self.orders, self.disputes
        )
    }
}

/// Rebuilds every projection, optionally the version tables with them.
///
/// Not wrapped in a single transaction: a rebuild of a full archive would
/// hold one open for as long as it runs, blocking the `sync` the operator
/// probably has running beside it. Each step is idempotent, so a rebuild that
/// dies halfway is fixed by running it again.
pub async fn rebuild(pool: &SqlitePool, from_raw: bool) -> Result<Rebuilt> {
    let mut rebuilt = Rebuilt::default();

    if from_raw {
        repo::orders::clear_versions(pool).await?;
        repo::disputes::clear_versions(pool).await?;
        repo::dev_fees::clear(pool).await?;
        repo::instance_info::clear(pool).await?;
    }

    repo::orders::clear_projection(pool).await?;
    repo::disputes::clear_projection(pool).await?;
    repo::instances::clear(pool).await?;

    // Everything the pipeline can be told is already in the archive, so the
    // policy admits nothing: `replay` does not consult it.
    let pipeline = Pipeline::new(pool.clone(), Policy::new(Vec::<String>::new(), false, []));

    for record in repo::events::all(pool).await?.iter() {
        let Ok(event) = Event::from_json(&record.raw_json) else {
            tracing::warn!(id = %record.id, "archived event is not readable JSON");
            rebuilt.unreadable += 1;
            continue;
        };

        match pipeline.replay(&event).await? {
            IngestOutcome::Rejected(_) => rebuilt.unreadable += 1,
            _ => rebuilt.events += 1,
        }
    }

    // The sweep, rather than trusting the replay to have touched everything:
    // a version whose raw event was lost is still a version, and its
    // projection is still owed.
    for order_id in repo::orders::ids(pool).await? {
        repo::orders::refresh_projection(pool, &order_id).await?;
        rebuilt.orders += 1;
    }
    for dispute_id in repo::disputes::ids(pool).await? {
        repo::disputes::refresh_projection(pool, &dispute_id).await?;
        rebuilt.disputes += 1;
    }
    for order_id in repo::dev_fees::order_ids(pool).await? {
        repo::dev_fees::refresh_duplicates(pool, &order_id).await?;
    }

    Ok(rebuilt)
}

/// `bestiario rebuild [--from-raw]`.
pub async fn run(context: &Context<'_>, from_raw: bool) -> Result<()> {
    let rebuilt = rebuild(context.pool, from_raw)
        .await
        .context("rebuilding the projections")?;

    println!("rebuild: {rebuilt}");

    Ok(())
}

#[cfg(test)]
mod tests;
