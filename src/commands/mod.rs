//! One module per CLI subcommand.
//!
//! Responsibility: wire configuration, database and relays together for a
//! single user-facing operation. Commands hold no domain logic of their own;
//! they assemble the layers below. See `docs/SPEC.md` §10.

use anyhow::{Context as _, Result};
use chrono::Utc;
use sqlx::SqlitePool;

pub mod backfill;
pub mod range;
pub mod rebuild;
pub mod stats;
pub mod sync;

use crate::cli::{Cli, Command, StatsCommand};
use crate::config::Settings;

/// What every command is handed: the validated configuration, an open and
/// migrated pool, and the invocation itself.
pub struct Context<'a> {
    pub settings: &'a Settings,
    pub pool: &'a SqlitePool,
    pub cli: &'a Cli,
}

/// Loads configuration, opens the database and runs the requested command.
pub async fn run(cli: &Cli) -> Result<()> {
    let settings =
        Settings::load(&cli.config).with_context(|| format!("loading {}", cli.config.display()))?;

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
        Command::Backfill { kind } => backfill::run(context, *kind, Utc::now().timestamp()).await,
        Command::Sync => sync::run(context).await,
        Command::Summary => not_yet("summary", 28),
        Command::Instances => not_yet("instances", 27),
        Command::Instance { .. } => not_yet("instance", 27),
        Command::Compare => not_yet("compare", 29),
        Command::Series { .. } => not_yet("series", 41),
        Command::Market { .. } => not_yet("market", 42),
        Command::Orders { .. } => not_yet("orders", 29),
        Command::Rebuild { from_raw } => rebuild::run(context, *from_raw).await,
        Command::Stats(stats) => match stats {
            StatsCommand::Orders { by } => {
                stats::orders::run(context, *by, Utc::now().timestamp()).await
            }
            StatsCommand::Volume { .. } => not_yet("stats volume", 33),
            StatsCommand::Market { .. } => not_yet("stats market", 36),
            StatsCommand::Timing { .. } => not_yet("stats timing", 37),
            StatsCommand::DevFees { by } => {
                stats::dev_fees::run(context, *by, Utc::now().timestamp()).await
            }
            StatsCommand::Disputes { by } => {
                stats::disputes::run(context, *by, Utc::now().timestamp()).await
            }
            StatsCommand::Rates { .. } => not_yet("stats rates", 38),
        },
    }
}

/// The CLI surface is complete from the first release so that the shape of the
/// tool is reviewable as a whole, but the commands arrive over several phases.
/// An unimplemented one says which roadmap entry will bring it rather than
/// panicking or, worse, printing an empty report.
fn not_yet(command: &str, roadmap_pr: u16) -> Result<()> {
    anyhow::bail!("`{command}` is not implemented yet; it arrives in roadmap PR {roadmap_pr}")
}
