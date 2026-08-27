//! `bestiario stats timing [--by fiat|method|kind|instance]`: the §6.4
//! figures and the §7 funnel. Wiring only, like [`super::orders`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::TimingDimension;
use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::timing::{self, Dimension};

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<TimingDimension>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, by.map(dimension), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.4 and §7 metrics for `query`, globally or once per slice.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    now: i64,
) -> Result<Report> {
    let window = Window::new(query.range.from(), query.range.until());
    // Every transition that ends in the window, and the current book; not
    // the history.
    let orders =
        load::activity::lifecycle_in(pool, &query.scope, window.from, window.until, now).await?;

    Ok(Report::new(
        query.range,
        timing::report(&orders, window, dimension, now),
        now,
    ))
}

fn dimension(by: TimingDimension) -> Dimension {
    match by {
        TimingDimension::Fiat => Dimension::Fiat,
        TimingDimension::Method => Dimension::Method,
        TimingDimension::Kind => Dimension::Kind,
        TimingDimension::Instance => Dimension::Instance,
    }
}

#[cfg(test)]
mod tests;
