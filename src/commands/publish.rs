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

use std::path::Path;

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use nostr_sdk::prelude::{Keys, ToBech32 as _};

use crate::commands::Context;
use crate::config::{AssumptionSettings, PublishSettings};
use crate::db::load::{self, Scope};
use crate::db::repo::events;
use crate::nostr::client::RelayClient;
use crate::nostr::{nip11, signer};
use crate::stats::bucket::Coverage;
use crate::stats::publish::index::Publisher;
use crate::stats::publish::size::{self, Ceiling, Measured};
use crate::stats::publish::snapshot::{Document, Snapshot};
use crate::stats::series::{Assumption, Data, Priced};

/// A snapshot, computed and weighed: everything a review needs and
/// everything the signer of the next row will be handed.
pub struct Publication {
    pub snapshot: Snapshot,
    pub index: Document,
    pub ceiling: Ceiling,
    pub relays_asked: usize,
    pub measured: Vec<Measured>,
}

impl Publication {
    /// The index last, as §7 requires of publication and as a listing
    /// should therefore read: the documents it names come first.
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.snapshot
            .documents
            .iter()
            .chain(std::iter::once(&self.index))
    }
}

/// Computes the snapshot, checks it against the ceiling, prints it and
/// writes it.
pub async fn run(context: &Context<'_>, dry_run: bool, out: Option<&Path>, now: i64) -> Result<()> {
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

    let publication = compute(
        context.pool,
        &context.settings.assumptions,
        &context.settings.publish,
        &context.settings.report.reference_currency,
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
    }

    Ok(())
}

/// A published document is the whole network's, and its address carries no
/// scope (§3), so a scoped run would sign network-wide addresses over one
/// instance's figures — a document that lies about what it is.
fn refuse_scoped(context: &Context<'_>) -> Result<()> {
    anyhow::ensure!(
        context.cli.instance.is_none() && context.cli.network.is_none(),
        "`publish` cannot be scoped with --instance or --network: a published document \
         addresses the whole network, and scoping one would sign a network-wide address \
         over an instance's figures"
    );
    Ok(())
}

/// The whole snapshot, from one reading of the archive.
pub async fn compute(
    pool: &SqlitePool,
    assumptions: &AssumptionSettings,
    settings: &PublishSettings,
    reference_currency: &str,
    now: i64,
) -> Result<Publication> {
    let scope = Scope::default();
    // The floor is the conservative one every report already uses — the
    // latest first event across the kinds a snapshot reads, not the
    // oldest event in the table. It is what nulls a series bucket (§6.3),
    // so stating anything earlier in the index would advertise coverage
    // the documents themselves withhold. The ceiling has no such duty and
    // is the plain extent.
    let coverage = Coverage::from_extent(
        events::earliest_created_at(pool, &crate::nostr::filters::INDEXED_KINDS, &scope).await?,
        events::latest_created_at(pool, &scope).await?,
    );
    let data = load(pool, assumptions, reference_currency, &coverage, now).await?;

    let snapshot = Snapshot::compute(&data, coverage, &snapshot_id(now), now);
    let index = snapshot.index(&publisher());

    let advertised = nip11::limits(&settings.relays).await;
    let ceiling = advertised.iter().fold(
        Ceiling::configured(settings.max_content_bytes),
        |ceiling, relay| match relay.max_content_length {
            Some(limit) => ceiling.and_relay(&relay.relay, limit),
            None => ceiling,
        },
    );

    let measured = size::measure(
        &snapshot
            .documents
            .iter()
            .cloned()
            .chain(std::iter::once(index.clone()))
            .collect::<Vec<_>>(),
    );

    Ok(Publication {
        snapshot,
        index,
        ceiling,
        relays_asked: advertised.len(),
        measured,
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
            abbreviated(&document.hash),
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
        "publishing {} documents to {} relay(s) as {}\n",
        publication.measured.len(),
        client.relays().len(),
        keys.public_key().to_bech32()?
    );

    let mut refusals = Vec::new();
    for document in &publication.snapshot.documents {
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
        .send(&signer::sign(&publication.index, run, keys))
        .await?;
    anyhow::ensure!(
        index.is_published(),
        "every document was published but the index was refused by every relay: \
         a client reading the previous index will not see this snapshot"
    );

    out.push_str(&format!(
        "snapshot {} published, index last\n",
        run.snapshot_id
    ));
    Ok(out)
}

/// Writes every document as `<d>.json`, with `:` folded to `-` so the
/// name is a filename on every filesystem. The static snapshot a site
/// serves before its relay connection is live.
fn write(publication: &Publication, directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("creating {}", directory.display()))?;

    for document in publication.documents() {
        let name = format!("{}.json", document.address.to_string().replace(':', "-"));
        let path = directory.join(&name);
        std::fs::write(&path, document.content())
            .with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
