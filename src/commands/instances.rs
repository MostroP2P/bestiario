//! `bestiario instances` and `bestiario instance <PUBKEY|NAME>`: the
//! bestiary of `docs/SPEC.md` §6.5 and the profile view of §6.10.
//!
//! Wiring only: the profiles are [`load::instances`], the figures are
//! [`stats::instances`], and the profile view is assembled from the same
//! loaders the `stats` families use, scoped to the one instance — except
//! the orders, which are loaded for the whole network once so the share
//! can be computed without a second read.

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::query::Query;
use crate::db::load::{self, Scope};
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::instances;

/// `bestiario instances`.
pub async fn list(context: &Context<'_>, now: i64) -> Result<()> {
    let query = Query::resolve(context, now).await?;
    let report = list_report(context.pool, &query, now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// `bestiario instance <PUBKEY|NAME>`.
pub async fn profile(context: &Context<'_>, instance: &str, now: i64) -> Result<()> {
    let query = Query::resolve_for(context, Some(instance), now).await?;
    let report = profile_report(context.pool, &query, now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// One block per instance in the scope.
pub async fn list_report(pool: &SqlitePool, query: &Query, now: i64) -> Result<Report> {
    let profiles = load::instances::profiles(pool, &query.scope).await?;
    let orders = load::activity::orders(pool, &query.scope).await?;
    let window = Window::new(query.range.from(), query.range.until());

    Ok(Report::new(
        query.range,
        instances::list(&profiles, &orders, window, now),
        now,
    ))
}

/// The profile of the instance `query` is scoped to, with its figures and
/// its share of the network.
///
/// `query.scope.pubkey` must be set: the caller resolved the argument.
pub async fn profile_report(pool: &SqlitePool, query: &Query, now: i64) -> Result<Report> {
    let pubkey = query
        .scope
        .pubkey
        .as_deref()
        .context("`instance` needs an instance to profile")?;

    let profile = load::instances::profiles(pool, &query.scope)
        .await?
        .into_iter()
        .next()
        .with_context(|| format!("no instance with pubkey {pubkey}"))?;

    // The whole network's orders, for the share; the instance's own are the
    // subset with its pubkey.
    let network_scope = Scope {
        pubkey: None,
        networks: query.scope.networks.clone(),
    };
    let network = load::activity::orders(pool, &network_scope).await?;
    let own: Vec<_> = network
        .iter()
        .filter(|order| order.pubkey == pubkey)
        .cloned()
        .collect();

    let fees = load::dev_fees::load(pool, &query.scope).await?;
    // Disputes cannot be narrowed to a network (they carry none), so under
    // `--network` the block is reported as missing rather than network-wide.
    let disputes = if query.network_narrowed {
        None
    } else {
        Some(load::disputes::load(pool, &query.scope).await?)
    };
    let window = Window::new(query.range.from(), query.range.until());

    Ok(Report::new(
        query.range,
        instances::profile(
            &profile,
            &own,
            &network,
            &fees,
            disputes.as_ref(),
            window,
            now,
        ),
        now,
    ))
}

#[cfg(test)]
mod tests;
