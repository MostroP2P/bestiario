//! `bestiario stats market [--by fiat|kind|instance]`: the §6.3 figures.
//! Wiring only, like [`super::orders`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::MarketDimension;
use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::market::{self, Dimension};

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<MarketDimension>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, by.map(dimension), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.3 metrics for `query`, globally or once per slice.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    now: i64,
) -> Result<Report> {
    let orders = load::activity::orders(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());

    Ok(Report::new(
        query.range,
        market::report(&orders, window, dimension),
        now,
    ))
}

fn dimension(by: MarketDimension) -> Dimension {
    match by {
        MarketDimension::Fiat => Dimension::Fiat,
        MarketDimension::Kind => Dimension::Kind,
        MarketDimension::Instance => Dimension::Instance,
    }
}

#[cfg(test)]
mod tests;
