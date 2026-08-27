//! `bestiario market <FIAT>`: view 5 of `docs/SPEC.md` §6.10. Wiring only,
//! plus the check the argument needs.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::market::fiat;

/// Resolves the flags, computes the view, prints it.
pub async fn run(context: &Context<'_>, fiat: &str, now: i64) -> Result<()> {
    let code = currency(fiat)?;
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, &code, now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.10 view 5 metrics for `fiat` over the query's window.
pub async fn report(pool: &SqlitePool, query: &Query, fiat: &str, now: i64) -> Result<Report> {
    let orders = load::activity::orders(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());

    Ok(Report::new(
        query.range,
        fiat::report(&orders, fiat, window, now),
        now,
    ))
}

/// The argument as a currency code: three ASCII letters, upper-cased, the
/// way the instances publish them in the `f` tag.
fn currency(argument: &str) -> Result<String> {
    anyhow::ensure!(
        argument.len() == 3 && argument.bytes().all(|byte| byte.is_ascii_alphabetic()),
        "`{argument}` is not a currency: a currency is a three-letter code such as ARS"
    );
    Ok(argument.to_ascii_uppercase())
}

#[cfg(test)]
mod tests;
