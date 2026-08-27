//! `bestiario orders <ORDER_ID>`: the lifecycle of one order, version by
//! version, and the dev fee it produced.
//!
//! Not windowed: an order is asked for by id, and every version it ever
//! had is the answer. The report's range is therefore the order's own span
//! — first version to last — rather than the default thirty days, which
//! would say the wrong thing.

use anyhow::{Context as _, Result};
use sqlx::SqlitePool;

use crate::commands::Context;
use crate::commands::range::Range;
use crate::db::repo::{dev_fees, orders};
use crate::ingest::parse::order::{self, FiatAmount};
use crate::report::{Format, Report};
use crate::stats::activity;
use crate::stats::lifecycle::{self, FeeSeen, Fiat, Version};

pub async fn run(context: &Context<'_>, order_id: &str, now: i64) -> Result<()> {
    let report = report(context.pool, order_id, now).await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// The lifecycle of `order_id`, or an error naming it when no version has
/// ever been seen — an empty report would read as an order with no history
/// rather than as a typo.
pub async fn report(pool: &SqlitePool, order_id: &str, now: i64) -> Result<Report> {
    let versions: Vec<Version> = orders::versions(pool, order_id)
        .await
        .with_context(|| format!("reading the versions of order {order_id}"))?
        .into_iter()
        .map(version)
        .collect();
    anyhow::ensure!(
        !versions.is_empty(),
        "no order with id `{order_id}` has been seen"
    );

    let fees: Vec<FeeSeen> = dev_fees::for_order(pool, order_id)
        .await?
        .into_iter()
        .map(|stored| FeeSeen {
            at: stored.fee.created_at,
            amount_sats: stored.fee.amount_sats,
            is_duplicate: stored.is_duplicate,
        })
        .collect();

    // Half-open, so the last version is inside it.
    let first = versions.first().map(|version| version.at);
    let last = versions.last().map(|version| version.at + 1);
    let span = Range::resolve(first, last, now)?;

    Ok(Report::new(
        span,
        lifecycle::report(order_id, &versions, &fees),
        now,
    ))
}

/// The parser's version as the view's; the two `match`es are where the
/// compiler checks that the vocabularies agree.
fn version(version: order::OrderVersion) -> Version {
    Version {
        at: version.created_at,
        status: match version.status {
            order::Status::Pending => activity::Status::Pending,
            order::Status::InProgress => activity::Status::InProgress,
            order::Status::Success => activity::Status::Success,
            order::Status::Canceled => activity::Status::Canceled,
        },
        direction: match version.direction {
            order::Direction::Buy => activity::Direction::Buy,
            order::Direction::Sell => activity::Direction::Sell,
        },
        fiat_code: version.fiat_code,
        amount_sats: version.amount_sats,
        fiat: match version.fiat {
            FiatAmount::Fixed(amount) => Fiat::Fixed(amount),
            FiatAmount::Range { min, max } => Fiat::Range { min, max },
        },
        premium: version.premium,
        expires_at: version.expires_at,
    }
}

#[cfg(test)]
mod tests;
