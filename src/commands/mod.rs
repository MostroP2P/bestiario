//! One module per CLI subcommand.
//!
//! Responsibility: wire configuration, database and relays together for a
//! single user-facing operation. Commands hold no domain logic of their own;
//! they assemble the layers below. See `docs/SPEC.md` §10.

use std::path::Path;

use anyhow::{Context as _, Result};
use chrono::Utc;
use sqlx::SqlitePool;

pub mod backfill;
pub mod compare;
pub mod instances;
pub mod market;
pub mod order;
pub mod publish;
pub mod query;
pub mod range;
pub mod rebuild;
mod relays;
pub mod series;
pub mod stats;
pub mod summary;
pub mod sync;

use crate::cli::{Cli, Command, StatsCommand};
use crate::config::{DEFAULT_CONFIG_PATH, Settings};
use crate::db::load::Scope;
use crate::stats::bucket::Coverage;

/// What every command is handed: the validated configuration, an open and
/// migrated pool, and the invocation itself.
pub struct Context<'a> {
    pub settings: &'a Settings,
    pub pool: &'a SqlitePool,
    pub cli: &'a Cli,
}

/// Loads configuration, opens the database and runs the requested command.
pub async fn run(cli: &Cli) -> Result<()> {
    // A missing file is tolerated only when no `--config` was given, where it
    // means "configured through the environment" — the shape of the container
    // deployment, which ships no settings.toml. A path that was typed is a
    // path someone means, default-valued or not, and a typo has to fail
    // rather than index with whatever the environment happens to hold.
    let settings = match &cli.config {
        Some(path) => Settings::load(path).with_context(|| format!("loading {}", path.display())),
        None => {
            let path = Path::new(DEFAULT_CONFIG_PATH);
            Settings::load_optional(path).with_context(|| format!("loading {}", path.display()))
        }
    }?;

    let pool = crate::db::connect_and_migrate(&settings.database.url)
        .await
        .with_context(|| format!("opening {}", settings.database.url))?;

    let context = Context {
        settings: &settings,
        pool: &pool,
        cli,
    };

    let result = dispatch(&context).await;

    // Close explicitly so WAL checkpointing happens before the process exits,
    // whether or not the command succeeded.
    pool.close().await;

    result
}

async fn dispatch(context: &Context<'_>) -> Result<()> {
    match &context.cli.command {
        Command::Backfill { kind } => backfill::run(context, *kind, now()).await,
        Command::Sync => sync::run(context).await,
        Command::Summary => summary::run(context, now()).await,
        Command::Instances => instances::list(context, now()).await,
        Command::Instance { instance } => instances::profile(context, instance, now()).await,
        Command::Compare => compare::run(context, now()).await,
        Command::Series { metric, by, split } => {
            series::run(context, metric, *by, *split, now()).await
        }
        Command::Market { fiat } => market::run(context, fiat, now()).await,
        Command::Orders { order_id } => order::run(context, order_id, now()).await,
        Command::Publish {
            dry_run,
            out,
            republish,
        } => publish::run(context, *dry_run, out.as_deref(), *republish, now()).await,
        Command::Rebuild { from_raw } => rebuild::run(context, *from_raw).await,
        Command::Stats(stats) => match stats {
            StatsCommand::Orders { by } => stats::orders::run(context, *by, now()).await,
            StatsCommand::Volume { by, convert_to } => {
                stats::volume::run(context, *by, convert_to.as_deref(), now()).await
            }
            StatsCommand::Market { by } => stats::market::run(context, *by, now()).await,
            StatsCommand::Timing { by } => stats::timing::run(context, *by, now()).await,
            StatsCommand::DevFees { by } => stats::dev_fees::run(context, *by, now()).await,
            StatsCommand::Disputes { by } => stats::disputes::run(context, *by, now()).await,
            StatsCommand::Rates { fiat } => {
                stats::rates::run(context, fiat.as_deref(), now()).await
            }
        },
    }
}

/// How far back the archive can speak for a report reading `kinds`.
///
/// Read once per report rather than assumed: a bucket before the first
/// event stored of the kinds it reads is a period nobody indexed, and
/// printing zeros for it would draw a flat line the network never
/// published (`bestiario_stats::bucket`).
///
/// `scope` is the report's own, the one its loader read with, so a
/// `--instance` report is told how far back *that* instance was indexed
/// rather than how far back anything was.
pub async fn coverage(pool: &SqlitePool, kinds: &[u16], scope: &Scope) -> Result<Coverage> {
    Ok(Coverage::from_earliest(
        crate::db::repo::events::earliest_created_at(pool, kinds, scope).await?,
    ))
}

/// The wall clock every command reports against.
///
/// `BESTIARIO_NOW`, a unix timestamp, overrides it. A testing hook, and
/// documented as one: the end-to-end suite runs the real binary and pins
/// its output, and figures such as "open now" or "silent for" are
/// functions of the clock — without a fixed one, the same corpus would
/// print a different report every day and nothing could be pinned.
fn now() -> i64 {
    std::env::var("BESTIARIO_NOW")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| Utc::now().timestamp())
}
