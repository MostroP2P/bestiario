//! `bestiario stats rates [--fiat FIAT]`: the §6.8 figures. Wiring only,
//! like [`super::orders`], plus the two checks the flags need.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load;
use crate::network::Network;
use crate::report::{Format, Report};
use crate::stats::rates::feeds;

/// Resolves the flags, computes the report, prints it.
pub async fn run(context: &Context<'_>, fiat: Option<&str>, now: i64) -> Result<()> {
    refuse_network_scope(context.cli.network)?;
    let code = fiat.map(currency).transpose()?;
    let query = Query::resolve(context, now).await?;
    let report = report(context.pool, &query, code.as_deref(), now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The §6.8 metrics: the state of every feed now, and for `fiat` the
/// disparity across the instances quoting it.
///
/// The window of `query` heads the report, as it does everywhere, but no
/// figure here is taken over it: a feed is a live thing, and §6.8 asks
/// what it says now.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    fiat: Option<&str>,
    now: i64,
) -> Result<Report> {
    let feeds = load::rates::feeds(pool, query.scope.pubkey.as_deref()).await?;

    Ok(Report::new(
        query.range,
        feeds::report(&feeds, fiat, now),
        now,
    ))
}

/// `--fiat` as a currency code: three ASCII letters, upper-cased, the way
/// the instances publish them in their snapshots.
fn currency(flag: &str) -> Result<String> {
    anyhow::ensure!(
        flag.len() == 3 && flag.bytes().all(|byte| byte.is_ascii_alphabetic()),
        "`--fiat {flag}`: a currency is a three-letter code such as USD"
    );
    Ok(flag.to_ascii_uppercase())
}

/// `--network` is refused rather than honoured in part, as in
/// [`super::disputes`]: a rate snapshot carries no network tag.
fn refuse_network_scope(network: Option<Network>) -> Result<()> {
    anyhow::ensure!(
        network.is_none(),
        "`stats rates` cannot be scoped with --network: rate snapshots (kind 30078) \
         carry no network tag, so a feed belongs to no one network"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
