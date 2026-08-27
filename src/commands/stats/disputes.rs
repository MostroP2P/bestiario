//! `bestiario stats disputes [--by status|initiator|instance|period]`: the
//! §6.7 figures. Wiring only, like [`super::orders`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::DisputeDimension;
use crate::commands::Context;
use crate::db::load;
use crate::ingest::parse;
use crate::network::Network;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::disputes::{self, Dimension};

use crate::commands::query::Query;

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<DisputeDimension>, now: i64) -> Result<()> {
    refuse_network_scope(context.cli.network)?;
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, by.map(dimension), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.7 metrics for `query`, globally or as `dimension` asks.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    now: i64,
) -> Result<Report> {
    let data = load::disputes::load(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());
    let metrics = disputes::report(
        &data,
        window,
        now,
        dimension,
        super::super::coverage(pool, &[parse::dispute::KIND]).await?,
    );

    Ok(Report::new(query.range, metrics, now))
}

/// `--network` is refused rather than honoured in part.
///
/// A dispute event carries no `network` tag, so no dispute can be placed on
/// one; a report that narrowed the orders and not the disputes would print
/// a dispute rate that divides every network's disputes by one network's
/// takers, under a heading that said otherwise.
pub fn refuse_network_scope(network: Option<Network>) -> Result<()> {
    anyhow::ensure!(
        network.is_none(),
        "`stats disputes` cannot be scoped with --network: dispute events (kind 38386) \
         carry no network tag, so disputes are counted across every indexed network"
    );
    Ok(())
}

fn dimension(by: DisputeDimension) -> Dimension {
    match by {
        DisputeDimension::Status => Dimension::Status,
        DisputeDimension::Initiator => Dimension::Initiator,
        DisputeDimension::Instance => Dimension::Instance,
        DisputeDimension::Period => Dimension::Month,
        DisputeDimension::Day => Dimension::Day,
    }
}

#[cfg(test)]
mod tests;
