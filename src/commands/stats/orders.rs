//! `bestiario stats orders [--by …]`: the activity figures of
//! `docs/SPEC.md` §6.1.
//!
//! The command owns nothing but the wiring. The figures are
//! [`stats::activity`], the rows are [`load::activity`], and the two formats
//! are [`Report`]; what is left here is the mapping from the flag the user
//! typed to the dimension the aggregation slices on, and the print.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::OrderDimension;
use crate::commands::Context;
use crate::db::load;
use crate::ingest::parse;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::activity::{self, Dimension};

use crate::commands::query::Query;

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, by: Option<OrderDimension>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, by.map(dimension), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.1 metrics for `query`, globally or once per slice of `dimension`.
///
/// Split from [`run`] so a test can assert on the [`Report`] rather than on
/// captured stdout, and so a later view (`summary`, `instance`) can reuse
/// the same computation without printing it.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    now: i64,
) -> Result<Report> {
    let orders = load::activity::orders(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());
    let metrics = activity::report(
        &orders,
        window,
        now,
        dimension,
        super::super::coverage(pool, &[parse::order::KIND], &query.scope).await?,
    );

    Ok(Report::new(query.range, metrics, now))
}

/// The CLI's vocabulary as the aggregation's.
///
/// `period` is the month: §6.1 defines its delta month over month, and a
/// finer bucket belongs to `series`, which exists to plot one.
fn dimension(by: OrderDimension) -> Dimension {
    match by {
        OrderDimension::Status => Dimension::Status,
        OrderDimension::Kind => Dimension::Kind,
        OrderDimension::Fiat => Dimension::Fiat,
        OrderDimension::Method => Dimension::Method,
        OrderDimension::Instance => Dimension::Instance,
        OrderDimension::Period => Dimension::Month,
        OrderDimension::Day => Dimension::Day,
        OrderDimension::Hour => Dimension::Hour,
        OrderDimension::Weekday => Dimension::Weekday,
    }
}

#[cfg(test)]
mod tests;
