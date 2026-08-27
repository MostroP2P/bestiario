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
//! # What is asked for, and in what order
//!
//! Two subscriptions, not one. A kind 30078 or 10002 carries no `y` tag, so
//! §8.1 step 4b admits it only from a publisher already seen publishing a
//! tagged event — and a relay answers one REQ listing six kinds in whatever
//! order it likes. So the tagged kinds are subscribed to first and read
//! until every relay reports EOSE; only then does the subscription that
//! includes the untagged ones go out. Without that, an instance's relay list
//! replayed ahead of its orders is turned away, and a rejection is neither
//! archived nor cursor-advanced: for a 30078, whose relay keeps only the
//! latest, that is a snapshot lost rather than one delayed.
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
//! The set it reattaches to is re-read as the run goes, not fixed at
//! startup: a relay list ingested an hour in names relays an instance
//! publishes to, and one of them may be the only place some of its events
//! are. A stored relay list that names a relay nothing is dialling rebuilds
//! the subscription there and then — see [`Sync::with_discovery`].
//!
//! # Shutting down
//!
//! There is nothing to flush. The pipeline advances the cursor inside the
//! same call that stores the event (§8.1 step 8), so an interrupted `sync`
//! has already recorded everything it stored. Shutdown closes the sockets and
//! reports the tally.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context as _, Result};
use nostr_sdk::prelude::{Filter, PublicKey, RelayUrl};
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::backfill::{authors, listed_instances};
use crate::commands::range::Range;
use crate::config::Settings;
use crate::db::repo;
use crate::ingest::parse::relay_list;
use crate::ingest::pipeline::UNTAGGED_KINDS;
use crate::ingest::{Counts, IngestOutcome, Pipeline, Policy};
use crate::nostr::client::{Incoming, RelayClient};
use crate::nostr::filters;

/// How long to wait before the first reconnection attempt.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

/// The ceiling the backoff doubles up to. A minute is long enough not to
/// hammer a relay that is down and short enough that a relay coming back is
/// noticed within one.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// How long [`Sync::prime`] waits for the relays to finish replaying the
/// tagged kinds.
///
/// Long enough for a relay with a backlog to get through it, short enough
/// that one relay which never sends EOSE does not keep a run from following
/// anything at all — going ahead is exactly what this code did before, and
/// no worse.
const PRIME_TIMEOUT: Duration = Duration::from_secs(30);

/// The kinds a subscription may ask for without the archive's help: every
/// indexed kind that carries a `y` tag.
///
/// Spelled as a filter over the two lists rather than as a third list, so a
/// kind added to either is not silently left out of this one.
const TAGGED_KINDS: [u16; 4] = tagged_kinds();

/// [`TAGGED_KINDS`], computed.
const fn tagged_kinds() -> [u16; 4] {
    let mut tagged = [0u16; 4];
    let mut found = 0;
    let mut index = 0;
    while index < filters::INDEXED_KINDS.len() {
        let kind = filters::INDEXED_KINDS[index];
        let mut untagged = false;
        let mut other = 0;
        while other < UNTAGGED_KINDS.len() {
            if UNTAGGED_KINDS[other] == kind {
                untagged = true;
            }
            other += 1;
        }
        if !untagged {
            tagged[found] = kind;
            found += 1;
        }
        index += 1;
    }
    assert!(found == 4, "the tagged kinds are the indexed ones bar two");
    tagged
}

/// The live follower: a connected client, a pipeline to feed, and the
/// database the cursors live in.
pub struct Sync<'a> {
    client: &'a mut RelayClient,
    pipeline: &'a Pipeline,
    pool: &'a SqlitePool,
    /// Empty means *any* author; see [`filters::for_kind`].
    authors: Vec<PublicKey>,
    /// The instances the operator listed, whatever `accept_unknown_instances`
    /// says — half of who an untagged kind may be asked about.
    listed: Vec<PublicKey>,
    overlap: i64,
    /// Present when the run should follow the relays it discovers; see
    /// [`Sync::with_discovery`].
    settings: Option<&'a Settings>,
}

impl<'a> Sync<'a> {
    pub fn new(client: &'a mut RelayClient, pipeline: &'a Pipeline, pool: &'a SqlitePool) -> Self {
        Self {
            client,
            pipeline,
            pool,
            authors: Vec::new(),
            listed: Vec::new(),
            overlap: 0,
            settings: None,
        }
    }

    pub fn with_authors(mut self, authors: Vec<PublicKey>) -> Self {
        self.authors = authors;
        self
    }

    /// The instances `[indexer].instances` names, which vouch for an
    /// untagged kind before this archive has seen them publish anything.
    pub fn with_listed(mut self, listed: Vec<PublicKey>) -> Self {
        self.listed = listed;
        self
    }

    /// Follows the relays this run discovers, not only those it started
    /// with.
    ///
    /// A `sync` is meant to run for months, and the connection set it was
    /// built from is a snapshot of the relay table at startup. A kind 10002
    /// arriving an hour in names a relay an instance publishes to — and
    /// possibly the only relay carrying some of what it publishes. Without
    /// this the client would go on dialling the startup set until somebody
    /// restarted the process.
    ///
    /// Left off, nothing is re-read and the run follows exactly the relays
    /// it was handed; that is what the tests want, and what
    /// `discover_relays = false` means.
    pub fn with_discovery(mut self, settings: &'a Settings) -> Self {
        self.settings = settings.nostr.discover_relays.then_some(settings);
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
            // Before reattaching, because a relay list ingested by the last
            // subscription may have named a relay nothing has dialled yet.
            self.rediscover().await?;
            self.client.reattach().await;

            // Step 4b of §8.1 judges the untagged kinds against the archive,
            // so they are asked for only once the tagged ones have been
            // replayed to their end; see [`Sync::prime`].
            match self.prime(&mut shutdown, &mut counts).await? {
                Priming::Shutdown => {
                    tracing::info!(%counts, "sync stopping");
                    return Ok(counts);
                }
                Priming::RelaysChanged => continue,
                Priming::Done => {}
            }

            // The cursors are read before the subscription is attempted, and
            // separately from it, because the two failures are not the same
            // failure. A relay that will not take a REQ is worth retrying; a
            // database that will not answer is not, and retrying it forever
            // would leave a `sync` spinning in the dark reporting nothing.
            let targets = self.targets(&filters::INDEXED_KINDS).await?;

            match self.client.subscribe(targets).await {
                Ok(mut subscription) => {
                    attempt = 0;
                    let stopped = loop {
                        tokio::select! {
                            () = &mut shutdown => break Interrupted::Shutdown,
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
                                    // Read before `record` consumes nothing
                                    // but reads clearer next to the event it
                                    // is about.
                                    let named_relays = matches!(outcome, IngestOutcome::Stored)
                                        && event.kind.as_u16() == relay_list::KIND;
                                    counts.record(&outcome);

                                    if named_relays && self.rediscover().await? {
                                        break Interrupted::RelaysChanged;
                                    }
                                }
                                None => {
                                    tracing::warn!("the relay stream ended");
                                    break Interrupted::StreamEnded;
                                }
                            },
                        }
                    };

                    // Closed by name rather than left to lapse: the sockets
                    // stay open in two of these three cases, and a relay
                    // holding a REQ nobody reads goes on sending it events.
                    self.client.close(subscription).await;

                    match stopped {
                        Interrupted::Shutdown => {
                            tracing::info!(%counts, "sync stopping");
                            return Ok(counts);
                        }
                        // Not a failure and not a reason to wait: the relays
                        // to follow changed, and the subscription is rebuilt
                        // over the wider set at once.
                        Interrupted::RelaysChanged => continue,
                        Interrupted::StreamEnded => {}
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

    /// Re-reads the connection set and hands it to the client, reporting
    /// whether it named a relay this run is not following.
    ///
    /// Nothing is dialled here — [`RelayClient::reattach`] does that — so
    /// this is cheap enough to ask after every relay list that lands, and
    /// the answer is what decides whether rebuilding the subscription is
    /// worth it. `false` whenever discovery is off, which is what leaves a
    /// run following exactly the relays it was handed.
    async fn rediscover(&mut self) -> Result<bool> {
        let Some(settings) = self.settings else {
            return Ok(false);
        };

        let now = chrono::Utc::now().timestamp();
        let set = super::relays::connection_set(self.pool, settings, now)
            .await
            .context("re-reading the relays to follow")?;
        self.client.reconfigure(&set);

        let waiting = self.client.unattached();
        if !waiting.is_empty() {
            tracing::info!(
                relays = waiting.len(),
                "a relay list named relays this run is not following"
            );
        }

        Ok(!waiting.is_empty())
    }

    /// Reads the tagged kinds to the end of what the relays have stored,
    /// before anything untagged is asked for.
    ///
    /// A kind 30078 or 10002 is admitted only from a publisher already seen
    /// publishing a `y = mostro` event ([`UNTAGGED_KINDS`]). `backfill` gets
    /// that ordering by walking kind by kind; a subscription cannot, because
    /// one REQ listing six kinds is answered in whatever order the relay
    /// likes. An instance's stored relay list replayed before its orders
    /// would be turned away — and a rejection is neither archived nor
    /// cursor-advanced, so the subscription that turned it away has no
    /// reason to send it again. For a 30078 that is not a delay but a loss:
    /// the relay keeps only the latest snapshot, and the one refused is gone
    /// when it is replaced.
    ///
    /// So the tagged kinds get a subscription of their own first, read until
    /// every relay has reported EOSE. Bounded by [`PRIME_TIMEOUT`]: a relay
    /// that never says it is done cannot hold up the run, and going ahead
    /// without it is no worse than the single subscription this replaces.
    ///
    /// [`UNTAGGED_KINDS`]: crate::ingest::pipeline::UNTAGGED_KINDS
    async fn prime<F: Future<Output = ()>>(
        &mut self,
        shutdown: &mut std::pin::Pin<&mut F>,
        counts: &mut Counts,
    ) -> Result<Priming> {
        let targets = self.targets(&TAGGED_KINDS).await?;
        let mut waiting: BTreeSet<RelayUrl> =
            targets.iter().map(|(relay, _)| relay.clone()).collect();

        let mut subscription = match self.client.subscribe(targets).await {
            Ok(subscription) => subscription,
            // The reconnection path deals with a relay that will not take a
            // REQ; there is nothing this one can add.
            Err(error) => {
                tracing::warn!(%error, "could not open the priming subscription");
                return Ok(Priming::Done);
            }
        };

        let deadline = tokio::time::Instant::now() + PRIME_TIMEOUT;
        let mut named_relays = false;
        let ending = loop {
            if waiting.is_empty() {
                break Priming::Done;
            }

            tokio::select! {
                () = &mut *shutdown => break Priming::Shutdown,
                next = tokio::time::timeout_at(deadline, subscription.next_incoming()) => {
                    match next {
                        Err(_) => {
                            tracing::warn!(
                                relays = waiting.len(),
                                seconds = PRIME_TIMEOUT.as_secs(),
                                "some relays never reported the end of their history; \
                                 asking for the untagged kinds anyway"
                            );
                            break Priming::Done;
                        }
                        Ok(None) => break Priming::Done,
                        Ok(Some(Incoming::EndOfStored(relay))) => {
                            waiting.remove(&relay);
                        }
                        Ok(Some(Incoming::Event(relay, event))) => {
                            let now = chrono::Utc::now().timestamp();
                            let outcome = self
                                .pipeline
                                .ingest(&event, &relay.to_string(), now)
                                .await
                                .with_context(|| {
                                    format!("storing event {} from {relay}", event.id)
                                })?;
                            named_relays |= matches!(outcome, IngestOutcome::Stored)
                                && event.kind.as_u16() == relay_list::KIND;
                            counts.record(&outcome);
                        }
                    }
                }
            }
        };

        self.client.close(subscription).await;

        // A relay list stored here is stored once and would look like a
        // duplicate ever after, so the set it named is checked now or not at
        // all. `named_relays` is what keeps this from repeating: an event
        // already in the archive cannot make it true a second time.
        if matches!(ending, Priming::Done) && named_relays && self.rediscover().await? {
            return Ok(Priming::RelaysChanged);
        }

        Ok(ending)
    }

    /// One filter set per relay, for `kinds`, each resumed from its own
    /// cursor.
    /// A relay left with no filter at all is left out rather than sent an
    /// empty REQ: before anything is vouched for, the untagged kinds have
    /// nobody to ask about, and asking every author for kind 10002 is a
    /// crawl of the network's whole NIP-65 index.
    async fn targets(&self, kinds: &[u16]) -> Result<Vec<(RelayUrl, Vec<Filter>)>> {
        let mut targets = Vec::new();
        let vouched = self.vouched().await?;

        for relay in self.client.relays().to_vec() {
            let mut per_relay = Vec::new();
            for &kind in kinds {
                if let Some(filter) = self.filter(&relay, kind, &vouched).await? {
                    per_relay.push(filter);
                }
            }
            if !per_relay.is_empty() {
                targets.push((relay, per_relay));
            }
        }

        Ok(targets)
    }

    /// Who an untagged kind may be asked about: the instances the operator
    /// listed, and the ones this archive has already seen publishing a
    /// `y = mostro` event.
    ///
    /// Re-read before every subscription, because the priming pass that
    /// precedes it proves publishers this one may then ask about — the same
    /// reason `backfill` re-reads it per kind.
    async fn vouched(&self) -> Result<Vec<PublicKey>> {
        let mut vouched = self.listed.clone();

        for pubkey in repo::instances::platform_proven(self.pool)
            .await
            .context("reading which publishers this archive vouches for")?
        {
            if let Ok(key) = PublicKey::parse(&pubkey)
                && !vouched.contains(&key)
            {
                vouched.push(key);
            }
        }

        Ok(vouched)
    }

    /// The filter for one `(relay, kind)`, resumed from its cursor, or
    /// `None` when this run must not ask that relay for that kind.
    async fn filter(
        &self,
        relay: &RelayUrl,
        kind: u16,
        vouched: &[PublicKey],
    ) -> Result<Option<Filter>> {
        let cursor = repo::sync_state::get(self.pool, &relay.to_string(), kind)
            .await
            .with_context(|| format!("reading the cursor for {relay} kind {kind}"))?
            .map(|cursor| cursor.last_created_at);

        let range = resume_from(cursor, self.overlap).map(Range::onwards);

        Ok(filters::for_kind(kind, &self.authors, vouched, range, None))
    }
}

/// How the priming subscription of [`Sync::prime`] ended.
enum Priming {
    /// Every relay reported the end of its stored history, or stopped being
    /// worth waiting for.
    Done,
    /// The caller asked the run to stop.
    Shutdown,
    /// A relay list read while priming named a relay this run is not
    /// following.
    RelaysChanged,
}

/// Why the inner loop of [`Sync::follow`] gave the subscription back.
///
/// Three endings that look alike from inside the loop and want three
/// different things afterwards: to return, to wait and reconnect, or to
/// resubscribe at once over a set of relays that has grown.
enum Interrupted {
    /// The caller asked the run to stop.
    Shutdown,
    /// The relay stream ended — a relay restarting, a network that went
    /// away.
    StreamEnded,
    /// A relay list named a relay this run is not following.
    RelaysChanged,
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

    let relays = super::relays::connection_set(context.pool, settings, super::now()).await?;
    let mut client = RelayClient::connect(&relays)
        .await
        .context("connecting to the relays")?;

    let pipeline = Pipeline::new(context.pool.clone(), Policy::from(&settings.indexer));
    let mut sync = Sync::new(&mut client, &pipeline, context.pool)
        .with_authors(authors(settings)?)
        .with_listed(listed_instances(settings)?)
        .with_overlap(settings.nostr.resume_overlap_secs as i64)
        .with_discovery(settings);

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
