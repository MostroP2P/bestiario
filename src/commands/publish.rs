//! `bestiario publish [--dry-run] [--out <dir>]`: the snapshot of
//! `docs/NOSTR-PUBLICATION.md` §12, computed and reviewed.
//!
//! Wiring, plus the three things wiring has to decide here: what the
//! archive's extent is, what ceiling every document is weighed against
//! (§9.1), and what a run that signs nothing is allowed to do.
//!
//! # Nothing is signed and nothing is published
//!
//! This is the reviewable half of publication. `--dry-run` prints what
//! would be published, with sizes and hashes; `--out` writes the same
//! documents as files, which is the static snapshot a site can serve
//! before its relay connection is live. Signing and relay publication
//! arrive with the key, and until then an invocation that asks for
//! neither is refused rather than quietly doing nothing.
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

use crate::commands::Context;
use crate::config::{AssumptionSettings, PublishSettings};
use crate::db::load::{self, Scope};
use crate::db::repo::events;
use crate::nostr::nip11;
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
    anyhow::ensure!(
        dry_run || out.is_some(),
        "`publish` signs nothing yet: pass --dry-run to review the snapshot, \
         or --out <dir> to write it as files"
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
