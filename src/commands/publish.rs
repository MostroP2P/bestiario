//! `bestiario publish [--dry-run] [--out <dir>]`: the snapshot of
//! `docs/NOSTR-PUBLICATION.md` §12, computed and reviewed.
//!
//! Wiring, plus the three things wiring has to decide here: what the
//! archive's extent is, what ceiling every document is weighed against
//! (§9.1), and what a run that signs nothing is allowed to do.
//!
//! # What a run does, and what it refuses to do
//!
//! `--dry-run` is the reviewable half: it prints what would be published,
//! with sizes and hashes, and signs nothing. `--out` writes the same
//! documents as files, which is the static snapshot a site can serve
//! before its relay connection is live. Anything else signs the snapshot
//! with the key of `[publish]` and sends it to `[publish].relays`.
//!
//! A run with no key and no `--dry-run` and no `--out` is refused rather
//! than quietly doing nothing: it is the invocation of an operator who
//! believes they have configured a publisher and has not.
//!
//! The index goes last (§7). An index names every document with the hash
//! of the payload that belongs to it, so an index on a relay is a promise
//! that the documents it names are already there — which is only true if
//! nothing is left to send when it goes out. A document no relay accepted
//! breaks that promise, so the index is not sent at all and the run
//! fails naming what was missing.
//!
//! # One pass over the archive
//!
//! Every document of a snapshot shares one `snapshot_id` and one reading
//! of the archive (§7), so the data is loaded once and handed to
//! `bestiario_stats::publish::snapshot`. Shelling out to the report
//! commands would give a snapshot whose documents disagree about what the
//! archive held.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use nostr_sdk::prelude::{Keys, ToBech32 as _};

use crate::commands::Context;
use crate::config::{AssumptionSettings, PublishSettings};
use crate::db::load::{self, Scope};
use crate::db::repo::{events, published};
use crate::nostr::client::RelayClient;
use crate::nostr::{nip11, signer};
use crate::stats::bucket::Coverage;
use crate::stats::publish::document::SCHEMA_VERSION;
use crate::stats::publish::index::{Index, Publisher};
use crate::stats::publish::restatement::{Because, Previous, Read, Republish, Restated};
use crate::stats::publish::size::{self, Ceiling, Measured};
use crate::stats::publish::snapshot::Snapshot;
use crate::stats::series::{Assumption, Data, Priced};
use crate::stats::window::Window;

/// A snapshot, computed and weighed: everything a review needs and
/// everything the signer of the next row will be handed.
pub struct Publication {
    pub snapshot: Snapshot,
    pub index: Index,
    pub ceiling: Ceiling,
    pub relays_asked: usize,
    pub measured: Vec<Measured>,
    /// The addresses this run does not send, because their payload is
    /// already published and no republication asked for them (§8). Still
    /// listed, still in the index, still recorded.
    pub not_sent: BTreeSet<String>,
    /// What this run leaves behind for the next one to compare against.
    pub state: BTreeMap<String, Previous>,
    /// How many events the archive held when this snapshot was computed.
    pub events: u64,
}

impl Publication {
    /// Every document as a `d` and the `content` published under it, the
    /// index last — the order §7 requires of publication, and the one a
    /// listing should therefore read in: the documents the index names
    /// come first. Pairs rather than one type because the index is not a
    /// [`Document`]; §6 exempts it from the envelope the rest carry.
    pub fn documents(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.snapshot
            .documents
            .iter()
            .map(|document| (document.address.to_string(), document.content()))
            .chain(std::iter::once((
                self.index.address().to_string(),
                self.index.content(),
            )))
    }
}

/// Computes the snapshot, checks it against the ceiling, prints it and
/// writes it.
pub async fn run(
    context: &Context<'_>,
    dry_run: bool,
    out: Option<&Path>,
    republish: bool,
    now: i64,
) -> Result<()> {
    refuse_scoped(context)?;
    let settings = &context.settings.publish;
    // Only a run that is going to sign asks for the key, so `--dry-run`
    // reviews a snapshot on a machine that holds none. And it asks before
    // the archive is read: a run that cannot sign should discover it in
    // the first second, not after thirty documents.
    let keys = match (dry_run, settings.nsec.as_ref()) {
        (false, Some(reference)) => Some(signer::resolve(reference)?),
        _ => None,
    };
    anyhow::ensure!(
        dry_run || out.is_some() || keys.is_some(),
        "`publish` has no signing key: point [publish].nsec at an environment \
         variable holding one, or pass --dry-run to review the snapshot, or \
         --out <dir> to write it as files"
    );

    // Only a run that is going to sign has a clock to answer for: a
    // review or a write to disk replaces nothing on a relay. Asked before
    // the archive is read, for the same reason the key is.
    if keys.is_some() {
        refuse_stalled_clock(published::latest_run(context.pool).await?.as_ref(), now)?;
    }

    let publication = compute(
        context.pool,
        &context.settings.assumptions,
        &context.settings.publish,
        &context.settings.report.reference_currency,
        requested(context.cli.from, context.cli.until, republish)?,
        now,
    )
    .await?;

    refuse_oversized(&publication)?;

    if dry_run {
        print!("{}", listing(&publication));
    }
    if let Some(directory) = out {
        write(&publication, directory)?;
    }

    if let Some(keys) = &keys {
        print!("{}", send(&publication, keys, &settings.relays).await?);
        // Written only after the relays took it. A run that recorded
        // first and then failed would tell the next run that documents
        // are published which are not, and the skip of §8 would leave
        // them missing until their figures happened to move.
        record(context.pool, &publication).await?;
    }

    Ok(())
}

/// What the invocation asked to be republished (§9.3).
///
/// The global `--from` / `--until` mean here what they mean everywhere: a
/// span of time. They select partitions rather than filter rows, which is
/// the only reading of "republish a range" that a partitioned format
/// allows.
fn requested(from: Option<i64>, until: Option<i64>, republish: bool) -> Result<Republish> {
    if !republish {
        return Ok(Republish::No);
    }
    match (from, until) {
        (None, None) => Ok(Republish::All),
        // One end given is a half-open range, which is what a recovery
        // usually is: everything since the relay was reset.
        (from, until) => {
            let window = Window {
                from: from.unwrap_or(0),
                until: until.unwrap_or(i64::MAX),
            };
            // Refused rather than obeyed. An empty or reversed range
            // overlaps no partition, so the run would send exactly what
            // an ordinary one sends, print nothing unusual and exit
            // zero — while the operator believes the history they asked
            // for is back on the relay. A recovery that silently
            // recovers nothing is worse than one that fails.
            anyhow::ensure!(
                window.from < window.until,
                "--republish over an empty range: --from {} is not before --until {}. \
                 Nothing overlaps it, so the run would republish nothing while appearing \
                 to succeed",
                window.from,
                window.until
            );
            Ok(Republish::Range(window))
        }
    }
}

/// Refuses a run whose clock has not passed the last publication's (§7).
///
/// A published document is replaceable, and every one of them carries the
/// run's `generated_at` as its `created_at` (§11). A relay keeps the copy
/// with the highest `created_at` and breaks a tie on event id, so a second
/// run inside the same second would sign replacements the relay has no
/// reason to prefer over what it already holds — and it would say so to
/// nobody: the send succeeds, the archive records a publication, and the
/// figures on the relay are the old ones. The same second also repeats
/// `snapshot_id`, which §7 wants unique per run.
///
/// The clock going backwards is the same fault with a longer reach — an
/// NTP correction can put a run behind the one before it for as long as
/// the step lasted — so the check is on the ordering, not on equality.
fn refuse_stalled_clock(last: Option<&published::Run>, now: i64) -> Result<()> {
    let Some(last) = last else {
        return Ok(());
    };
    anyhow::ensure!(
        now > last.generated_at,
        "the clock has not advanced past the last publication: this run is timestamped {} \
         and snapshot {} was published at {}. Every document carries that timestamp, and a \
         relay keeps the copy with the later one, so this run would replace nothing while \
         reporting success. Wait until the clock passes it and run again",
        crate::stats::publish::document::rfc3339(now),
        last.snapshot_id,
        crate::stats::publish::document::rfc3339(last.generated_at)
    );
    Ok(())
}

/// Records what this run published, so the next one can say what changed.
async fn record(pool: &SqlitePool, publication: &Publication) -> Result<()> {
    // One transaction, because the two halves are one fact. The next run
    // reads the documents to decide each revision and the run to decide
    // *why* the figures moved; a crash between them would leave some
    // documents at this run's revision, the rest at the last one, and the
    // run row still naming the publication before. The next run would
    // then skip documents it should send and re-issue others under a
    // revision already used, with a restatement clock from the wrong run
    // — and §8's history is not a thing a later run can repair.
    let mut tx = pool.begin().await.context("recording the publication")?;

    for (address, previous) in &publication.state {
        published::record(&mut *tx, address, previous)
            .await
            .with_context(|| format!("recording the publication of {address}"))?;
    }

    published::record_run(
        &mut *tx,
        &published::Run {
            snapshot_id: publication.snapshot.run.snapshot_id.clone(),
            generated_at: publication.snapshot.run.generated_at,
            schema_version: SCHEMA_VERSION,
            first_event_at: publication.snapshot.coverage.earliest(),
            last_event_at: publication.snapshot.coverage.latest(),
            events: publication.events,
        },
    )
    .await
    .context("recording the publication run")?;

    tx.commit().await.context("recording the publication")?;

    Ok(())
}

/// A snapshot decides its own scopes (§3): the network-wide documents and
/// one `orders` document per instance, every one of them computed from the
/// same reading of the whole archive. Narrowing the *run* would not narrow
/// the addresses it signs — it would sign `orders:30d` over one instance's
/// orders, and `orders:30d:i:<other>` over an archive that no longer holds
/// that instance's. A document that lies about what it is, either way.
fn refuse_scoped(context: &Context<'_>) -> Result<()> {
    anyhow::ensure!(
        context.cli.instance.is_none() && context.cli.network.is_none(),
        "`publish` cannot be scoped with --instance or --network: a snapshot decides its \
         own scopes — the network-wide documents and one per instance — and narrowing the \
         run would sign those addresses over a subset of the archive"
    );
    Ok(())
}

/// The whole snapshot, from one reading of the archive.
pub async fn compute(
    pool: &SqlitePool,
    assumptions: &AssumptionSettings,
    settings: &PublishSettings,
    reference_currency: &str,
    republish: Republish,
    now: i64,
) -> Result<Publication> {
    let scope = Scope::default();
    // The floor is the conservative one every report already uses — the
    // latest first event across the kinds a snapshot reads, not the
    // oldest event in the table. It is what nulls a series bucket (§6.3),
    // so stating anything earlier in the index would advertise coverage
    // the documents themselves withhold. The ceiling has no such duty and
    // is the plain extent.
    //
    // Read before the figures, and that order is load-bearing. These are
    // separate reads, so an ingest running alongside can land an event
    // between them; taking the extent first means the figures can only be
    // a superset of what the index claims, never a subset. A run that
    // loaded first could state a floor below data it does not have and
    // publish the flat line at zero §6.3 exists to prevent. The surplus
    // is harmless in the other direction: an event newer than the ceiling
    // either falls in a bucket already covered, or in one no partition
    // was computed for, and the next run picks it up.
    let coverage = Coverage::from_extent(
        events::earliest_created_at(pool, &crate::nostr::filters::INDEXED_KINDS, &scope).await?,
        events::latest_created_at(pool, &scope).await?,
    );
    let data = load(pool, assumptions, reference_currency, &coverage, now).await?;

    // What was published before, and why the figures moved since (§8).
    // The reason is read off the archive rather than off a flag: `publish`
    // is not told whether a backfill or a rebuild ran before it, but the
    // archive records enough to say.
    let history = published::all(pool).await?;
    let last = published::latest_run(pool).await?;
    let held = events::count(pool).await?;
    let current = Read {
        schema_version: SCHEMA_VERSION,
        covered_from: coverage.earliest(),
        events: held,
    };
    // With no run to compare against, nothing has a revision above the
    // first and no reason is published; the current reading stands in for
    // the previous one, so the comparison is trivially "nothing moved"
    // rather than an arbitrary reason.
    let because = Because::inferred(
        last.as_ref().map_or(current, |run| Read {
            schema_version: run.schema_version,
            covered_from: run.first_event_at,
            events: run.events,
        }),
        current,
    );

    let computed = Snapshot::compute(&data, coverage, &snapshot_id(now), now);
    let Restated {
        snapshot,
        not_sent,
        state,
    } = computed.restated(&history, because, republish);

    // The index is not under §8's skip: nothing hashes it, and naming the
    // current snapshot is its whole job, so it is republished on every
    // run by definition (§5). It is built after the rest because it is
    // built *from* the rest — including the revisions just decided.
    let index = snapshot.index(&publisher());

    let advertised = nip11::limits(&settings.relays).await;
    let ceiling = advertised.iter().fold(
        Ceiling::configured(settings.max_content_bytes),
        |ceiling, relay| match relay.max_content_length {
            Some(limit) => ceiling.and_relay(&relay.relay, limit),
            None => ceiling,
        },
    );

    let mut measured = size::measure(&snapshot.documents);
    measured.push(size::measure_index(&index));

    Ok(Publication {
        snapshot,
        index,
        ceiling,
        relays_asked: advertised.len(),
        measured,
        not_sent,
        state,
        events: held,
    })
}

/// Everything the four families read, over the whole archive: a snapshot
/// publishes all of them, so loading per family would read the orders
/// three times.
async fn load(
    pool: &SqlitePool,
    assumptions: &AssumptionSettings,
    reference_currency: &str,
    coverage: &Coverage,
    now: i64,
) -> Result<Data> {
    let scope = Scope::default();
    let from = coverage.earliest().unwrap_or(now);

    Ok(Data {
        orders: load::activity::orders(pool, &scope).await?,
        fees: load::dev_fees::load(pool, &scope).await?,
        disputes: load::disputes::load(pool, &scope).await?,
        // The `instances` and `compare` documents are about the instances
        // themselves, and are the only place a client learns that an
        // instance exists at all.
        profiles: load::instances::profiles(pool, &scope).await?,
        dev_fee_pct: Some(Assumption {
            per_instance: assumptions.dev_fee_percentage.clone(),
            default: assumptions.dev_fee_percentage_default,
        }),
        priced: Some(Priced {
            book: load::rates::book(pool, from, now)
                .await
                .context("loading the rate snapshots")?,
            code: reference_currency.to_string(),
        }),
    })
}

/// Who signed a snapshot, as the index names them.
fn publisher() -> Publisher {
    Publisher {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// The `snapshot_id` of §7: monotonic, unique per run, and derived from
/// the run's own clock so that two runs over the same archive are
/// distinguishable by when they happened.
///
/// A compact UTC timestamp rather than a random identifier because it is
/// also a provenance record a human reads — "which run last computed
/// this" — and because a snapshot that is a function of the archive and
/// the clock should be a function of them all the way down: a random one
/// would make an otherwise reproducible run irreproducible.
pub fn snapshot_id(now: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(now, 0)
        .map(|at| at.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|| now.to_string())
}

/// A document over the ceiling is an error naming it, never a silent
/// rejection by one relay after the rest of the snapshot is already
/// published (§9.1).
fn refuse_oversized(publication: &Publication) -> Result<()> {
    let over = size::over(&publication.measured, &publication.ceiling);
    if over.is_empty() {
        return Ok(());
    }

    let named = over
        .iter()
        .map(|document| format!("  {} is {} bytes", document.address, document.bytes))
        .collect::<Vec<_>>()
        .join("\n");

    anyhow::bail!(
        "{} document(s) exceed the {}-byte ceiling{}:\n{named}",
        over.len(),
        publication.ceiling.bytes(),
        match publication.ceiling.relay() {
            Some(relay) => format!(" advertised by {relay}"),
            None => " of [publish].max_content_bytes".to_string(),
        }
    )
}

/// What `--dry-run` prints: the run, the extent it rests on, the ceiling
/// in force, and every document with its size and its payload hash.
///
/// The relays are counted rather than listed: a listing is compared
/// against a configuration file the reader already has, and only a relay
/// that actually lowers the ceiling changes what happens.
pub fn listing(publication: &Publication) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "snapshot {} generated at {}\n",
        publication.snapshot.run.snapshot_id,
        crate::stats::publish::document::rfc3339(publication.snapshot.run.generated_at)
    ));
    out.push_str(&match (
        publication.snapshot.coverage.earliest(),
        publication.snapshot.coverage.latest(),
    ) {
        (Some(first), Some(last)) => format!(
            "archive {} to {}\n",
            crate::stats::publish::document::rfc3339(first),
            crate::stats::publish::document::rfc3339(last)
        ),
        // Said rather than left out: the window documents below are full
        // of zeros, and this is the line that says why (§5).
        _ => "archive holds nothing\n".to_string(),
    });
    out.push_str(&format!(
        "ceiling {} bytes ({}), {} relay(s) asked\n\n",
        publication.ceiling.bytes(),
        publication
            .ceiling
            .relay()
            .map_or_else(|| "[publish].max_content_bytes".to_string(), str::to_string),
        publication.relays_asked
    ));

    let width = publication
        .measured
        .iter()
        .map(|document| document.address.to_string().len())
        .max()
        .unwrap_or(1);
    for document in &publication.measured {
        out.push_str(&format!(
            "{:<width$}  {:>7}  {}\n",
            document.address.to_string(),
            document.bytes,
            document.hash.as_deref().map_or_else(
                // Nothing hashes the index (§6), so there is no digest to
                // abbreviate and a dash says so, as it does everywhere
                // else a figure is absent.
                || "—".to_string(),
                abbreviated,
            ),
            width = width
        ));
    }

    let total: usize = publication.measured.iter().map(|d| d.bytes).sum();
    out.push_str(&format!(
        "\n{} documents, {total} bytes\n",
        publication.measured.len()
    ));
    out
}

/// A hash as §5 writes them in prose: enough to tell two apart at a
/// glance, in a listing meant to be read. The whole digest is what
/// `--out` writes and what the index carries; a review is not a
/// comparison a person does 64 characters at a time.
fn abbreviated(hash: &str) -> String {
    format!("{}…", &hash[..HASH_PREFIX.min(hash.len())])
}

/// Characters of a hash a listing shows.
const HASH_PREFIX: usize = 16;

/// Signs every document and sends it, the index last (§7).
///
/// Returns what happened rather than printing as it goes: the run either
/// completes and is reported in one piece, or fails on a document nobody
/// took and reports that instead. A half-printed listing followed by an
/// error reads like the error belongs to the last line printed.
async fn send(publication: &Publication, keys: &Keys, relays: &[String]) -> Result<String> {
    let client = RelayClient::connect(relays).await?;
    let report = send_to(publication, keys, &client).await;
    // Closed whether the run succeeded or failed on a document nobody
    // took: an aborted publication should not leave a websocket open
    // behind it either.
    client.shutdown().await;
    report
}

/// The publication itself, over relays somebody else connected.
///
/// Split from [`send`] because a relay that answered at connection time
/// and is gone by the time a document is sent is exactly the case §7 is
/// about, and it cannot be reached through a function that dials its own
/// relays.
async fn send_to(publication: &Publication, keys: &Keys, client: &RelayClient) -> Result<String> {
    let run = &publication.snapshot.run;
    let mut out = format!(
        "publishing to {} relay(s) as {}\n",
        client.relays().len(),
        keys.public_key().to_bech32()?
    );

    let mut refusals = Vec::new();
    let mut sent = 0;
    for document in &publication.snapshot.documents {
        // §8: a payload already on the relay is not re-signed and not
        // sent. It stays in the index with the hash, revision and clock
        // it already had — "unchanged" is one of the things the index
        // exists to say.
        if publication.not_sent.contains(&document.address.to_string()) {
            continue;
        }
        sent += 1;
        let delivery = client.send(&signer::sign(document, run, keys)).await?;
        for (relay, reason) in &delivery.refused {
            out.push_str(&format!(
                "  {} refused by {relay}: {reason}\n",
                document.address
            ));
        }
        if !delivery.is_published() {
            refusals.push(document.address.to_string());
        }
    }

    // Before the index, not after: an index that named a document no
    // relay holds would send readers to fetch something that is not
    // there, and would go on doing so until the next run.
    anyhow::ensure!(
        refusals.is_empty(),
        "{} document(s) reached no relay, so the index naming them was not published:\n{}",
        refusals.len(),
        refusals
            .iter()
            .map(|address| format!("  {address}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let index = client
        .send(&signer::sign_index(&publication.index, run, keys))
        .await?;
    anyhow::ensure!(
        index.is_published(),
        "every document was published but the index was refused by every relay: \
         a client reading the previous index will not see this snapshot"
    );

    // The index is always one of them, and is not counted as a document
    // whose figures did or did not move: it has none of its own.
    out.push_str(&format!(
        "snapshot {} published: {sent} document(s) sent, {} unchanged, index last\n",
        run.snapshot_id,
        publication.not_sent.len()
    ));
    Ok(out)
}

/// Writes every document as `<d>.json`, with `:` folded to `-` so the
/// name is a filename on every filesystem. The static snapshot a site
/// serves before its relay connection is live.
fn write(publication: &Publication, directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    for (address, content) in publication.documents() {
        let name = format!("{}.json", address.replace(':', "-"));
        let path = directory.join(&name);
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
