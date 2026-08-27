//! `bestiario stats volume [--by kind|fiat|instance|period] [--in CUR]`:
//! the §6.2 figures. Wiring only, like [`super::orders`].

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::VolumeDimension;
use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::volume::{self, Dimension};

/// Resolves the flags, computes the report, prints it.
pub async fn run(
    context: &Context<'_>,
    by: Option<VolumeDimension>,
    convert_to: Option<&str>,
    now: i64,
) -> Result<()> {
    anyhow::ensure!(
        convert_to.is_none(),
        "`--in` is not implemented yet; it arrives in roadmap PR 35"
    );
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, by.map(dimension), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The observed §6.2 metrics for `query`, globally or once per slice.
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
        volume::report(&orders, window, dimension),
        now,
    ))
}

fn dimension(by: VolumeDimension) -> Dimension {
    match by {
        VolumeDimension::Kind => Dimension::Kind,
        VolumeDimension::Fiat => Dimension::Fiat,
        VolumeDimension::Instance => Dimension::Instance,
        VolumeDimension::Period => Dimension::Month,
    }
}

#[cfg(test)]
mod tests;
