//! `bestiario sync`: follow the relays live (`docs/SPEC.md` §8.2).
//!
//! # Where a subscription resumes from
//!
//! Each `(relay, kind)` has its own cursor, and each is rewound by
//! `resume_overlap_secs` before being asked for. Relays disagree about the
//! time — an event is stamped by its publisher, not by the relay — so
//! resuming exactly at the cursor would drop anything that arrived stamped
//! slightly earlier than the last event seen. Rewinding re-reads a little,
//! and the dedup step of §8.1 absorbs it.
//!
//! A relay with no cursor is asked for everything it holds. That is the
//! first run, and the stored events a subscription replays before going live
//! are exactly what a backfill would have fetched.
//!
//! # Reconnecting
//!
//! When the stream ends — a relay restarting, a network that went away — the
//! subscription is rebuilt after a backoff that doubles up to a minute.
//! Cursors are re-read at that point, so a reconnection resumes from what was
//! actually stored rather than from where the process started.
//!
//! Every subscription is preceded by [`RelayClient::reattach`], so a relay
//! that was down when the indexer started rejoins the run once it answers.
//! `connect` drops what does not answer, and a `sync` meant to run for months
//! cannot be the reason a relay stays ignored until somebody restarts it.
//!
//! # Shutting down
//!
//! There is nothing to flush. The pipeline advances the cursor inside the
//! same call that stores the event (§8.1 step 8), so an interrupted `sync`
//! has already recorded everything it stored. Shutdown closes the sockets and
//! reports the tally.

use std::time::Duration;

use anyhow::{Context as _, Result};
use nostr_sdk::prelude::{Filter, PublicKey, RelayUrl};
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::backfill::authors;
use crate::commands::range::Range;
use crate::db::repo;
use crate::ingest::{Counts, Pipeline, Policy};
use crate::nostr::client::RelayClient;
use crate::nostr::filters;

/// How long to wait before the first reconnection attempt.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

/// The ceiling the backoff doubles up to. A minute is long enough not to
/// hammer a relay that is down and short enough that a relay coming back is
/// noticed within one.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// The live follower: a connected client, a pipeline to feed, and the
/// database the cursors live in.
pub struct Sync<'a> {
    client: &'a mut RelayClient,
    pipeline: &'a Pipeline,
    pool: &'a SqlitePool,
    /// Empty means *any* author; see [`filters::for_kind`].
    authors: Vec<PublicKey>,
    overlap: i64,
}

impl<'a> Sync<'a> {
    pub fn new(client: &'a mut RelayClient, pipeline: &'a Pipeline, pool: &'a SqlitePool) -> Self {
        Self {
            client,
            pipeline,
            pool,
            authors: Vec::new(),
            overlap: 0,
        }
    }

    pub fn with_authors(mut self, authors: Vec<PublicKey>) -> Self {
        self.authors = authors;
        self
    }

    /// How far each cursor is rewound before resuming.
    pub fn with_overlap(mut self, seconds: i64) -> Self {
        self.overlap = seconds;
        self
    }

    /// Follows every connected relay until `shutdown` completes.
    ///
    /// Returns what the run stored. The `Err` case is the database being
    /// unusable: a relay that fails is reconnected to, not returned.
    pub async fn follow(&mut self, shutdown: impl Future<Output = ()>) -> Result<Counts> {
        let mut counts = Counts::default();
        let mut attempt = 0u32;

        let mut shutdown = std::pin::pin!(shutdown);

        loop {
            self.client.reattach().await;

            // The cursors are read before the subscription is attempted, and
            // separately from it, because the two failures are not the same
            // failure. A relay that will not take a REQ is worth retrying; a
            // database that will not answer is not, and retrying it forever
            // would leave a `sync` spinning in the dark reporting nothing.
            let targets = self.targets().await?;

            match self.client.subscribe(targets).await {
                Ok(mut subscription) => {
                    attempt = 0;
                    loop {
                        tokio::select! {
                            () = &mut shutdown => {
                                tracing::info!(%counts, "sync stopping");
                                return Ok(counts);
                            }
                            next = subscription.next_event() => match next {
                                Some((relay, event)) => {
                                    let now = chrono::Utc::now().timestamp();
                                    let outcome = self
                                        .pipeline
                                        .ingest(&event, &relay.to_string(), now)
                                        .await
                                        .with_context(|| {
                                            format!("storing event {} from {relay}", event.id)
                                        })?;
                                    counts.record(&outcome);
                                }
                                None => {
                                    tracing::warn!("the relay stream ended");
                                    break;
                                }
                            },
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "could not subscribe"),
            }

            attempt += 1;
            let wait = backoff(attempt);
            tracing::info!(attempt, seconds = wait.as_secs(), "reconnecting");

            tokio::select! {
                () = &mut shutdown => return Ok(counts),
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// One filter set per relay, each kind resumed from its own cursor.
    async fn targets(&self) -> Result<Vec<(RelayUrl, Vec<Filter>)>> {
        let mut targets = Vec::new();

        for relay in self.client.relays().to_vec() {
            let mut per_relay = Vec::new();
            for &kind in &filters::INDEXED_KINDS {
                per_relay.push(self.filter(&relay, kind).await?);
            }
            targets.push((relay, per_relay));
        }

        Ok(targets)
    }

    /// The filter for one `(relay, kind)`, resumed from its cursor.
    async fn filter(&self, relay: &RelayUrl, kind: u16) -> Result<Filter> {
        let cursor = repo::sync_state::get(self.pool, &relay.to_string(), kind)
            .await
            .with_context(|| format!("reading the cursor for {relay} kind {kind}"))?
            .map(|cursor| cursor.last_created_at);

        let range = resume_from(cursor, self.overlap).map(Range::onwards);

        Ok(filters::for_kind(kind, &self.authors, range, None))
    }
}

/// Where a subscription resumes: the cursor rewound by `overlap`, or nothing
/// at all when the cursor has never been set.
///
/// Clamped at zero, because a filter asking for events published before the
/// epoch says nothing a relay can act on, and the rewind of a cursor near
/// zero would otherwise go negative.
fn resume_from(cursor: Option<i64>, overlap: i64) -> Option<i64> {
    cursor.map(|cursor| (cursor - overlap).max(0))
}

/// How long to wait before reconnection attempt `attempt`, doubling from
/// [`BACKOFF_INITIAL`] up to [`BACKOFF_MAX`].
fn backoff(attempt: u32) -> Duration {
    let doublings = attempt.saturating_sub(1).min(u32::BITS - 1);
    BACKOFF_INITIAL
        .saturating_mul(1u32 << doublings)
        .min(BACKOFF_MAX)
}

/// `bestiario sync`.
///
/// Runs until the process is interrupted. SIGINT is a normal way to stop an
/// indexer, so it exits reporting what it stored rather than as a failure.
pub async fn run(context: &Context<'_>) -> Result<()> {
    let settings = context.settings;

    let mut client = RelayClient::connect(&settings.nostr.relays)
        .await
        .context("connecting to the configured relays")?;

    let pipeline = Pipeline::new(context.pool.clone(), Policy::from(&settings.indexer));
    let mut sync = Sync::new(&mut client, &pipeline, context.pool)
        .with_authors(authors(settings)?)
        .with_overlap(settings.nostr.resume_overlap_secs as i64);

    let counts = sync
        .follow(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "cannot listen for SIGINT; sync will not stop cleanly");
                std::future::pending::<()>().await;
            }
        })
        .await?;

    client.shutdown().await;

    println!("sync: {counts}");

    Ok(())
}

#[cfg(test)]
mod tests;
