//! `bestiario stats disputes [--by status|initiator|instance|period]`: the
//! §6.7 figures. Wiring only, like [`super::orders`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::DisputeDimension;
use crate::commands::Context;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::disputes::{self, Dimension};

use crate::commands::query::Query;

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<DisputeDimension>, now: i64) -> Result<()> {
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
    let metrics = disputes::report(&data, window, now, dimension);

    Ok(Report::new(query.range, metrics, now))
}

fn dimension(by: DisputeDimension) -> Dimension {
    match by {
        DisputeDimension::Status => Dimension::Status,
        DisputeDimension::Initiator => Dimension::Initiator,
        DisputeDimension::Instance => Dimension::Instance,
        DisputeDimension::Period => Dimension::Month,
    }
}

#[cfg(test)]
mod tests;
