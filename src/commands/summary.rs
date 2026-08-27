//! `bestiario summary`: view 1 of `docs/SPEC.md` §6.10. Wiring only.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::summary;

pub async fn run(context: &Context<'_>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

pub async fn report(pool: &SqlitePool, query: &Query, now: i64) -> Result<Report> {
    let orders = load::activity::orders(pool, &query.scope).await?;
    // Disputes carry no network tag: under `--network` the count is missing.
    let disputes = if query.network_narrowed {
        None
    } else {
        Some(load::disputes::load(pool, &query.scope).await?)
    };
    let window = Window::new(query.range.from(), query.range.until());

    Ok(Report::new(
        query.range,
        summary::report(&orders, disputes.as_ref(), window, now),
        now,
    ))
}

#[cfg(test)]
mod tests;
