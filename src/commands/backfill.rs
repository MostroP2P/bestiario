//! `bestiario backfill`: walk relay history backwards until there is no more
//! of it (`docs/SPEC.md` §8.2).
//!
//! # Why backwards
//!
//! A relay answers a REQ with the *newest* events matching the filter, capped
//! at the limit. Walking forwards from the floor would therefore ask for the
//! oldest window and be handed the newest events in it — the same page, over
//! and over. Stepping the `until` bound down to the oldest event of each
//! window is what makes the next request return something new.
//!
//! # Where it stops
//!
//! - an empty window, which is the relay saying it holds nothing older;
//! - reaching the floor the caller gave (`--from`, or `backfill_from`).
//!
//! A window that came back with room to spare is *not* one of them. The limit
//! in a REQ is what the client asked for, not what the relay promised: a relay
//! that caps its replies at a hundred events answers a request for five
//! hundred with a page that looks unfinished and is not. Reading that as the
//! end of the history would stop the walk at the newest hundred events and
//! report a complete backfill. Only a window the relay answers with nothing
//! at all says there is nothing older, so that is what the walk waits for —
//! at the cost of one more round trip and one deduplicated event per walk.
//!
//! # The overlapping second
//!
//! Each step sets the next window to end *at* the oldest event just seen,
//! not below it, because a relay may hold several events in that second and
//! stepping below would skip the rest of them. The one event that repeats is
//! absorbed by the dedup step of §8.1 and costs a row that is not written.
//!
//! That overlap cannot make progress when a whole page fits inside a single
//! second — a relay holding more events in that second than one page carries.
//! The walk then steps strictly below it, and says so in the log when the page
//! came back full: losing the rest of one second is bad, and looping on it
//! forever is worse.

use anyhow::{Context as _, Result};
use nostr_sdk::prelude::{Event, PublicKey, RelayUrl};
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::range::Range;
use crate::config::Settings;
use crate::db::repo;
use crate::ingest::{Counts, Pipeline, Policy};
use crate::nostr::client::RelayClient;
use crate::nostr::filters;

/// How many events one window asks a relay for.
///
/// Large enough that a busy day fits in few round trips, small enough that a
/// relay is not asked to assemble a reply it will refuse to send.
const WINDOW_LIMIT: usize = 500;

/// The walk: a connected client, a pipeline to feed, and who to ask about.
pub struct Backfill<'a> {
    client: &'a RelayClient,
    pipeline: &'a Pipeline,
    /// Read to decide who is vouched for; see [`Backfill::vouched`].
    pool: &'a SqlitePool,
    /// Empty means *any* author, which is what `accept_unknown_instances`
    /// asks for; see [`filters::for_kind`].
    authors: Vec<PublicKey>,
    /// The instances the operator listed, whatever `accept_unknown_instances`
    /// says — half of who an untagged kind may be asked about.
    listed: Vec<PublicKey>,
    window_limit: usize,
}

impl<'a> Backfill<'a> {
    pub fn new(client: &'a RelayClient, pipeline: &'a Pipeline, pool: &'a SqlitePool) -> Self {
        Self {
            client,
            pipeline,
            pool,
            authors: Vec::new(),
            listed: Vec::new(),
            window_limit: WINDOW_LIMIT,
        }
    }

    /// Narrows every request to these authors.
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

    /// Overrides how many events a window asks for. Tests use it to force the
    /// walk to page; nothing else has a reason to.
    pub fn with_window_limit(mut self, limit: usize) -> Self {
        self.window_limit = limit;
        self
    }

    /// Walks every connected relay, for every kind, over `range`.
    ///
    /// A relay that stops answering is logged and the walk moves on to the
    /// next one, because a run that gave up on the first timeout would leave
    /// the other relays unread for no reason.
    ///
    /// A database that stops answering is the opposite case and is returned:
    /// once writes are failing every later window fails too, and the archive
    /// the summary would claim is on disk is not there. Worse, an event whose
    /// write failed is not retried by the next run — §8.1 reads the archive
    /// row first — so a run that swallowed the error would report a number it
    /// cannot back up and exit zero.
    pub async fn run(&self, kinds: &[u16], range: Range, now: i64) -> Result<Counts> {
        let counts = self
            .walk_relays(self.client.relays(), kinds, range, now)
            .await?;

        tracing::info!(
            stored = counts.stored,
            duplicate = counts.duplicate,
            rejected = counts.rejected,
            "backfill finished"
        );

        Ok(counts)
    }

    /// The same walk over the relays named, rather than over every relay the
    /// client holds.
    ///
    /// What a discovery round needs: a relay this run has already walked is
    /// not worth walking twice, and a relay that a kind 10002 named halfway
    /// through is worth walking now rather than at the next invocation.
    pub async fn walk_relays(
        &self,
        relays: &[RelayUrl],
        kinds: &[u16],
        range: Range,
        now: i64,
    ) -> Result<Counts> {
        let mut counts = Counts::default();

        for relay in relays {
            for &kind in kinds {
                counts += self.walk(relay, kind, range, now).await?;
            }
        }

        Ok(counts)
    }

    /// Who an untagged kind may be asked about right now: the instances the
    /// operator listed, and the ones this archive has already seen
    /// publishing a `y = mostro` event.
    ///
    /// Read per kind rather than once per run, because the run itself
    /// changes the answer: [`filters::INDEXED_KINDS`] walks the tagged kinds
    /// first, and each one proves publishers the untagged kinds may then be
    /// asked about. A first backfill on an empty archive therefore still
    /// discovers relay lists — from the instances its own orders have just
    /// vouched for.
    async fn vouched(&self) -> Result<Vec<PublicKey>> {
        let mut vouched = self.listed.clone();

        for pubkey in repo::instances::platform_proven(self.pool)
            .await
            .context("reading which publishers this archive vouches for")?
        {
            // A pubkey that reached a table came off a verified event, so
            // this cannot fail; ignoring it rather than failing the walk is
            // still the right shape for a filter that is only a narrowing.
            if let Ok(key) = PublicKey::parse(&pubkey)
                && !vouched.contains(&key)
            {
                vouched.push(key);
            }
        }

        Ok(vouched)
    }

    /// One relay, one kind, from the top of `range` down to its floor.
    async fn walk(&self, relay: &RelayUrl, kind: u16, range: Range, now: i64) -> Result<Counts> {
        let mut counts = Counts::default();
        let mut until = range.until();
        let vouched = self.vouched().await?;

        while until > range.from() {
            let window = match Range::resolve(Some(range.from()), Some(until), now) {
                Ok(window) => window,
                // The floor and the ceiling have met: the walk is done.
                Err(_) => break,
            };

            let Some(filter) = filters::for_kind(
                kind,
                &self.authors,
                &vouched,
                Some(window),
                Some(self.window_limit),
            ) else {
                // An untagged kind with nobody to ask about: see
                // `filters::for_kind`. The tagged kinds walked before this
                // one found no instance, so there is no narrowed request to
                // make and a broad one is not this indexer's to make.
                tracing::debug!(%relay, kind, "no vouched publisher; not asking for this kind");
                break;
            };

            let events = match self.client.fetch_window(relay, filter).await {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(%relay, kind, %error, "window failed; leaving this relay here");
                    break;
                }
            };

            if events.is_empty() {
                tracing::debug!(%relay, kind, "no more history");
                break;
            }

            let full_page = events.len() >= self.window_limit;
            let oldest = Self::oldest(&events);
            counts += self.ingest_all(&events, relay, now).await?;

            tracing::info!(
                %relay,
                kind,
                events = events.len(),
                until,
                oldest,
                "window walked"
            );

            // Step to the oldest second seen, keeping it, so that events
            // sharing it are not skipped. `oldest < until` always — the relay
            // only returns events below the bound — so the walk always moves,
            // and the strict step below is what keeps it moving when a whole
            // page fits inside one second.
            let next = oldest + 1;
            until = if next < until {
                next
            } else {
                if full_page {
                    // Only a page the relay filled to the brim is evidence
                    // that the second was cut short. A shorter page ending in
                    // the same second is the relay running out of events, and
                    // the empty window that follows confirms it.
                    tracing::warn!(
                        %relay,
                        kind,
                        second = oldest,
                        limit = self.window_limit,
                        "a full page fits inside one second; stepping past it, \
                         events published in that second may be missed"
                    );
                }
                oldest
            };
        }

        Ok(counts)
    }

    /// Feeds one window to the pipeline, oldest first.
    ///
    /// The first write that fails ends the run. `SQLITE_BUSY`, a full disk or
    /// a projection that will not refresh are not conditions the next event
    /// recovers from, and an operator reading a summary needs it to be a
    /// count of what is on disk.
    ///
    /// Order matters for the projections: a version arriving after a newer one
    /// is handled correctly by `refresh_projection`, but replaying in
    /// publication order keeps the intermediate states the projection passes
    /// through the ones the network really went through.
    async fn ingest_all(&self, events: &[Event], relay: &RelayUrl, now: i64) -> Result<Counts> {
        let mut counts = Counts::default();
        let relay_url = relay.to_string();

        for event in events.iter().rev() {
            let outcome = self
                .pipeline
                .ingest(event, &relay_url, now)
                .await
                .with_context(|| format!("storing event {} from {relay}", event.id))?;
            counts.record(&outcome);
        }

        Ok(counts)
    }

    /// The `created_at` of the oldest event in a window.
    ///
    /// Read rather than assumed: the window arrives newest first, but a relay
    /// is free to answer in any order and the next window's bound depends on
    /// this being right.
    fn oldest(events: &[Event]) -> i64 {
        events
            .iter()
            .map(|event| event.created_at.as_secs() as i64)
            .min()
            .unwrap_or_default()
    }
}

/// The authors to narrow relay requests to, from the configuration.
///
/// Empty when `accept_unknown_instances` is set: the point of that flag is to
/// index instances nobody has listed yet, and an author-filtered request
/// would never see them. The platform filter of §8.1 step 4 is what keeps
/// that from indexing the rest of NIP-69.
pub fn authors(settings: &Settings) -> anyhow::Result<Vec<PublicKey>> {
    if settings.indexer.accept_unknown_instances {
        return Ok(Vec::new());
    }

    listed_instances(settings)
}

/// The instances `[indexer].instances` names, whatever
/// `accept_unknown_instances` says.
///
/// Not the same question as [`authors`]: that one is who a *request* may be
/// narrowed to, and the flag widens it to anybody. This one is who the
/// operator vouches for by name, which no flag widens — an untagged kind is
/// asked about these and about whoever the archive has since proven, and
/// about nobody else.
pub fn listed_instances(settings: &Settings) -> anyhow::Result<Vec<PublicKey>> {
    settings
        .indexer
        .instances
        .iter()
        .map(|pubkey| {
            PublicKey::parse(pubkey)
                .map_err(|error| anyhow::anyhow!("[indexer].instances: `{pubkey}`: {error}"))
        })
        .collect()
}

/// `bestiario backfill [--kind K] [--from T] [--until T]`.
///
/// The floor defaults to `[indexer].backfill_from` rather than to the
/// thirty-day window the reporting commands use: a backfill with a
/// report's default would quietly stop a month back and leave the archive
/// looking complete.
pub async fn run(context: &Context<'_>, kind: Option<u16>, now: i64) -> Result<()> {
    let settings = context.settings;
    let kinds = requested_kinds(kind)?;

    let from = context.cli.from.unwrap_or(settings.indexer.backfill_from);
    let range = Range::resolve(Some(from), context.cli.until, now)?;

    let relays = super::relays::connection_set(context.pool, settings, now).await?;
    let mut client = RelayClient::connect(&relays)
        .await
        .context("connecting to the relays")?;

    let pipeline = Pipeline::new(context.pool.clone(), Policy::from(&settings.indexer));
    let counts = walk_with_discovery(context, &mut client, &pipeline, &kinds, range, now).await;

    // Disconnected whatever the walk decided: leaving sockets open behind a
    // failed run would keep the relays holding a subscription nobody reads.
    client.shutdown().await;

    report(context, &counts?, range);

    Ok(())
}

/// The walk, and then the relays the walk itself found.
///
/// The connection set is read before the first window and is out of date by
/// the time the last one lands: a kind 10002 ingested along the way names a
/// relay this run has not dialled, and treating the startup snapshot as
/// final would leave whatever only that relay carries unindexed until
/// somebody ran `backfill` a second time. So each round re-reads the set,
/// dials what is new and walks *only* what the earlier rounds did not.
///
/// The rounds end when one discovers nothing, which is the normal case and
/// usually the second round. [`MAX_DISCOVERY_ROUNDS`] is the backstop: relay
/// lists can name relays that carry relay lists, and one invocation of
/// `backfill` should terminate whatever the network advertises.
///
/// With `discover_relays` off there is nothing to find — `connection_set`
/// returns the configured relays and nothing else — so the walk is the
/// single round it always was.
async fn walk_with_discovery(
    context: &Context<'_>,
    client: &mut RelayClient,
    pipeline: &Pipeline,
    kinds: &[u16],
    range: Range,
    now: i64,
) -> Result<Counts> {
    let settings = context.settings;
    let authors = authors(settings)?;
    let listed = listed_instances(settings)?;
    let mut counts = Backfill::new(client, pipeline, context.pool)
        .with_authors(authors.clone())
        .with_listed(listed.clone())
        .run(kinds, range, now)
        .await?;

    if !settings.nostr.discover_relays {
        return Ok(counts);
    }

    for round in 1..=MAX_DISCOVERY_ROUNDS {
        let set = super::relays::connection_set(context.pool, settings, now).await?;
        client.reconfigure(&set);
        if client.unattached().is_empty() {
            break;
        }

        // `reattach` dials what is new and keeps what is already in play, so
        // the relays it adds are exactly the ones this run has not walked.
        let before = client.relays().to_vec();
        client.reattach().await;
        let fresh: Vec<RelayUrl> = client
            .relays()
            .iter()
            .filter(|relay| !before.contains(relay))
            .cloned()
            .collect();

        if fresh.is_empty() {
            break;
        }

        tracing::info!(
            round,
            relays = fresh.len(),
            "walking the relays this run discovered"
        );
        counts += Backfill::new(client, pipeline, context.pool)
            .with_authors(authors.clone())
            .with_listed(listed.clone())
            .walk_relays(&fresh, kinds, range, now)
            .await?;

        if round == MAX_DISCOVERY_ROUNDS {
            tracing::warn!(
                rounds = MAX_DISCOVERY_ROUNDS,
                "stopping discovery here; run backfill again to follow what is left"
            );
        }
    }

    Ok(counts)
}

/// How many times one `backfill` will follow a relay list to relays it has
/// not walked. Two rounds cover a configured relay naming an instance's own,
/// which is the shape the network actually has; the rest is a bound on a
/// chain nobody should be able to make endless.
const MAX_DISCOVERY_ROUNDS: usize = 3;

/// The kinds one invocation walks: the one asked for, or all of them.
///
/// A `--kind` bestiario has no parser for is refused with the list of the
/// ones it has, rather than accepted into a walk that could only fill the
/// rejected counter.
fn requested_kinds(kind: Option<u16>) -> Result<Vec<u16>> {
    let Some(kind) = kind else {
        return Ok(filters::INDEXED_KINDS.to_vec());
    };

    anyhow::ensure!(
        filters::INDEXED_KINDS.contains(&kind),
        "--kind {kind} is not indexed; bestiario reads {}",
        filters::INDEXED_KINDS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );

    Ok(vec![kind])
}

/// The one line a run leaves behind, as a table row or as JSON.
fn report(context: &Context<'_>, counts: &Counts, range: Range) {
    if context.cli.json {
        let (from, until) = range.to_rfc3339();
        println!(
            "{}",
            serde_json::json!({
                "range": { "from": from, "until": until },
                "events": {
                    "stored": counts.stored,
                    "duplicate": counts.duplicate,
                    "rejected": counts.rejected,
                    "total": counts.total(),
                }
            })
        );
    } else {
        println!("backfill: {counts}");
    }
}

#[cfg(test)]
mod tests;
