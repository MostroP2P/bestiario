//! `bestiario series <METRIC> [--by day|week|month|year] [--split …]`:
//! one metric over time (SPEC §6.10 view 4).
//!
//! Wiring, plus the two things wiring has to decide: which of the four
//! families to load — a series reads one, and loading the other three would
//! be three reads nobody asked for — and what to say when the metric named
//! does not exist, which is the list of the ones that do.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::cli::{Period as CliPeriod, SeriesSplit};
use crate::commands::Context;
use crate::commands::query::Query;
use crate::config::AssumptionSettings;
use crate::db::load;
use crate::report::{Format, Report};
use crate::stats::Window;
use crate::stats::series::{self, Assumption, Data, Family, SeriesError, Split};
use crate::stats::window::Period;

/// Resolves the flags, computes the series, prints it.
pub async fn run(
    context: &Context<'_>,
    metric: &str,
    by: CliPeriod,
    split: Option<SeriesSplit>,
    now: i64,
) -> Result<()> {
    let family = Family::of(metric);
    if family == Some(Family::Disputes) {
        super::stats::disputes::refuse_network_scope(context.cli.network)?;
    }
    let query = Query::resolve(context, now).await?;
    let report = report(
        context.pool,
        &query,
        &context.settings.assumptions,
        metric,
        period(by),
        split.map(dimension),
        now,
    )
    .await?;

    print!("{}", report.render(Format::from_flag(context.cli.json)));

    Ok(())
}

/// `metric` over the buckets of the query's window.
pub async fn report(
    pool: &SqlitePool,
    query: &Query,
    assumptions: &AssumptionSettings,
    metric: &str,
    period: Period,
    split: Option<Split>,
    now: i64,
) -> Result<Report> {
    let data = load_for(pool, query, assumptions, Family::of(metric)).await?;
    let window = Window::new(query.range.from(), query.range.until());

    let metrics = series::report(&data, window, period, metric, split, now)
        .map_err(|error| explain(error, &data, now))?;

    Ok(Report::new(query.range, metrics, now))
}

/// Only what the metric's family reads. With no family — an unknown metric
/// — everything, so that the error can list what this archive could plot.
async fn load_for(
    pool: &SqlitePool,
    query: &Query,
    assumptions: &AssumptionSettings,
    family: Option<Family>,
) -> Result<Data> {
    let orders = matches!(family, None | Some(Family::Activity) | Some(Family::Volume));
    let fees = matches!(family, None | Some(Family::DevFees));
    let disputes = matches!(family, None | Some(Family::Disputes));

    Ok(Data {
        orders: if orders {
            load::activity::orders(pool, &query.scope).await?
        } else {
            Vec::new()
        },
        fees: if fees {
            load::dev_fees::load(pool, &query.scope).await?
        } else {
            Default::default()
        },
        disputes: if disputes {
            load::disputes::load(pool, &query.scope).await?
        } else {
            Default::default()
        },
        dev_fee_pct: Some(Assumption {
            per_instance: assumptions.dev_fee_percentage.clone(),
            default: assumptions.dev_fee_percentage_default,
        }),
    })
}

/// An unknown metric is answered with the ones this archive can plot; the
/// other refusals already say everything they need to.
fn explain(error: SeriesError, data: &Data, now: i64) -> anyhow::Error {
    match error {
        SeriesError::UnknownMetric { .. } => {
            let known = series::catalogue(data, now).join(", ");
            anyhow::anyhow!("{error}; the metrics that can be are: {known}")
        }
        other => anyhow::anyhow!(other),
    }
}

fn period(by: CliPeriod) -> Period {
    match by {
        CliPeriod::Day => Period::Day,
        CliPeriod::Week => Period::Week,
        CliPeriod::Month => Period::Month,
        CliPeriod::Year => Period::Year,
    }
}

fn dimension(split: SeriesSplit) -> Split {
    match split {
        SeriesSplit::Instance => Split::Instance,
        SeriesSplit::Kind => Split::Kind,
        SeriesSplit::Fiat => Split::Fiat,
    }
}

#[cfg(test)]
mod tests;
