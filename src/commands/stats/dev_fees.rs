//! `bestiario stats dev-fees [--by instance|period]`: the §6.6 figures.
//!
//! Wiring only, like [`super::orders`]: the figures are
//! [`stats::dev_fees`], the rows are [`load::dev_fees`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::InstanceOrPeriod;
use crate::commands::Context;
use crate::config::AssumptionSettings;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::dev_fees::{self, Dimension};

use crate::commands::query::Query;

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<InstanceOrPeriod>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = report(
        context.pool,
        &query,
        by.map(dimension),
        &context.settings.assumptions,
        now,
    )
    .await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.6 metrics for `query`, globally or once per slice of `dimension`;
/// the implied volume rests on the dev fee share `assumptions` give each
/// instance.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    assumptions: &AssumptionSettings,
    now: i64,
) -> Result<Report> {
    let data = load::dev_fees::load(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());
    let pct = |pubkey: &str| assumptions.dev_fee_percentage_for(pubkey);
    let metrics = dev_fees::report(&data, window, dimension, &pct);

    Ok(Report::new(query.range, metrics, now))
}

fn dimension(by: InstanceOrPeriod) -> Dimension {
    match by {
        InstanceOrPeriod::Instance => Dimension::Instance,
        InstanceOrPeriod::Period => Dimension::Month,
    }
}

#[cfg(test)]
mod tests;
