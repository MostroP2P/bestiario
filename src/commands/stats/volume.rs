//! `bestiario stats volume [--by kind|fiat|instance|period] [--in CUR]`:
//! the §6.2 figures. Wiring only, like [`super::orders`] — plus the one
//! check the flags need, that `--in` names a currency code.

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use crate::cli::VolumeDimension;
use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::ingest::parse;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::volume::{self, Conversion, Dimension};

/// Resolves the flags, computes the report, prints it.
pub async fn run(
    context: &Context<'_>,
    by: Option<VolumeDimension>,
    convert_to: Option<&str>,
    now: i64,
) -> Result<()> {
    let code = convert_to.map(currency).transpose()?;
    let query = Query::resolve(context, now).await?;
    let report = report(
        context.pool,
        &query,
        by.map(dimension),
        code.as_deref(),
        now,
    )
    .await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.2 metrics for `query`, globally or once per slice: the observed
/// ones, and with `convert_to` the inferred conversion into that currency
/// after them, priced from every rate snapshot the database holds.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    dimension: Option<Dimension>,
    convert_to: Option<&str>,
    now: i64,
) -> Result<Report> {
    let window = Window::new(query.range.from(), query.range.until());
    // Only what the window can count: every §6.2 figure is over the orders
    // that reached `success` inside it, and a month slice is inside it too.
    let orders =
        load::activity::completed_in(pool, &query.scope, window.from, window.until).await?;
    let book = match convert_to {
        Some(_) => Some(
            load::rates::book(pool, window.from, window.until)
                .await
                .context("loading the rate snapshots")?,
        ),
        None => None,
    };
    let conversion = book
        .as_ref()
        .zip(convert_to)
        .map(|(book, code)| Conversion { book, code });

    Ok(Report::new(
        query.range,
        volume::report(
            &orders,
            window,
            dimension,
            conversion,
            super::super::coverage(pool, &[parse::order::KIND], &query.scope).await?,
        ),
        now,
    ))
}

/// `--in` as a currency code: three ASCII letters, upper-cased, the way
/// the instances publish them in their snapshots and their orders.
fn currency(flag: &str) -> Result<String> {
    anyhow::ensure!(
        flag.len() == 3 && flag.bytes().all(|byte| byte.is_ascii_alphabetic()),
        "`--in {flag}`: a currency is a three-letter code such as USD"
    );
    Ok(flag.to_ascii_uppercase())
}

fn dimension(by: VolumeDimension) -> Dimension {
    match by {
        VolumeDimension::Kind => Dimension::Kind,
        VolumeDimension::Fiat => Dimension::Fiat,
        VolumeDimension::Instance => Dimension::Instance,
        VolumeDimension::Period => Dimension::Month,
        VolumeDimension::Day => Dimension::Day,
    }
}

#[cfg(test)]
mod tests;
