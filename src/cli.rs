//! The command-line surface of `docs/SPEC.md` §10.
//!
//! Responsibility: parsing and validating what the user typed, and nothing
//! else. Every command's behaviour lives in [`crate::commands`]; this module
//! only decides whether the invocation is well formed and turns it into typed
//! values.
//!
//! Dimensions are `ValueEnum`s rather than free strings, so `--by fiatt` fails
//! at the argument parser with the list of accepted values instead of
//! producing an empty report.

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::network::Network;

/// Statistics for the Mostro network, indexed from public Nostr events.
#[derive(Debug, Parser)]
#[command(name = "bestiario", version, about, long_about = None)]
pub struct Cli {
    /// Path to settings.toml.
    #[arg(long, short = 'c', global = true, default_value = "settings.toml")]
    pub config: PathBuf,

    /// Emit JSON instead of a table.
    #[arg(long, global = true)]
    pub json: bool,

    /// Start of the reporting window: a unix timestamp or YYYY-MM-DD (UTC).
    #[arg(long, global = true, value_parser = parse_timestamp, value_name = "TIME")]
    pub from: Option<i64>,

    /// End of the reporting window: a unix timestamp or YYYY-MM-DD (UTC).
    #[arg(long, global = true, value_parser = parse_timestamp, value_name = "TIME")]
    pub until: Option<i64>,

    /// Restrict to one instance, by pubkey or by name.
    #[arg(long, global = true, value_name = "PUBKEY|NAME")]
    pub instance: Option<String>,

    /// Restrict to one network, overriding the configured list.
    #[arg(long, global = true, value_enum, value_name = "NETWORK")]
    pub network: Option<Network>,

    /// Increase log verbosity; repeat for more. Overridden by RUST_LOG.
    #[arg(long, short = 'v', global = true, action = ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Walk backwards through relay history, storing what is found.
    Backfill {
        /// Only this kind, rather than every kind bestiario indexes.
        #[arg(long, value_name = "KIND")]
        kind: Option<u16>,
    },

    /// Follow the relays live, storing events as they arrive.
    Sync,

    /// Network summary for the reporting window.
    Summary,

    /// Every known instance, with its profile.
    Instances,

    /// One instance: profile, its metrics and its share of the network.
    Instance {
        /// Pubkey or name.
        #[arg(value_name = "PUBKEY|NAME")]
        instance: String,
    },

    /// One row per instance, side by side.
    Compare,

    /// A metric over time.
    Series {
        /// Metric to plot, for example `orders.completed` or `volume.sats`.
        #[arg(value_name = "METRIC")]
        metric: String,

        /// Bucket size.
        #[arg(long, value_enum, default_value_t = Period::Month)]
        by: Period,

        /// Break each bucket down by this dimension.
        #[arg(long, value_enum)]
        split: Option<SeriesSplit>,
    },

    /// The market for one fiat currency.
    Market {
        /// ISO currency code, for example ARS.
        #[arg(value_name = "FIAT")]
        fiat: String,
    },

    /// Metric families, with free slicing.
    #[command(subcommand)]
    Stats(StatsCommand),

    /// The lifecycle of one order, version by version.
    Orders {
        /// Order UUID, as published in the `d` tag.
        #[arg(value_name = "ORDER_ID")]
        order_id: String,
    },

    /// Regenerate the projections from the stored versions.
    Rebuild {
        /// Also regenerate the version tables from the raw events.
        #[arg(long)]
        from_raw: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum StatsCommand {
    /// Activity: created, completed, canceled, rates (SPEC §6.1).
    ///
    /// Without `--by` the nine figures are reported for the whole window;
    /// with it, once per slice.
    Orders {
        #[arg(long, value_enum)]
        by: Option<OrderDimension>,
    },

    /// Traded volume in sats and fiat (SPEC §6.2).
    Volume {
        #[arg(long, value_enum, default_value_t = VolumeDimension::Fiat)]
        by: VolumeDimension,

        /// Convert into this currency. Inferred, and reported as such.
        #[arg(long = "in", value_name = "CURRENCY")]
        convert_to: Option<String>,
    },

    /// Market structure: pressure, premium, concentration (SPEC §6.3).
    Market {
        #[arg(long, value_enum, default_value_t = MarketDimension::Fiat)]
        by: MarketDimension,
    },

    /// How long each stage of an order takes (SPEC §6.4).
    Timing {
        #[arg(long, value_enum, default_value_t = TimingDimension::Fiat)]
        by: TimingDimension,
    },

    /// Dev fees sent to the development fund (SPEC §6.6).
    DevFees {
        #[arg(long, value_enum, default_value_t = InstanceOrPeriod::Instance)]
        by: InstanceOrPeriod,
    },

    /// Disputes by status, initiator and outcome (SPEC §6.7).
    Disputes {
        #[arg(long, value_enum, default_value_t = DisputeDimension::Status)]
        by: DisputeDimension,
    },

    /// Exchange rate feeds and their freshness (SPEC §6.8).
    Rates {
        /// Only this currency.
        #[arg(long, value_name = "FIAT")]
        fiat: Option<String>,
    },
}

/// The dimensions of SPEC §6, split per family so that each command offers
/// only the slices that mean something for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OrderDimension {
    Status,
    Kind,
    Fiat,
    Method,
    Instance,
    /// Calendar months inside the window.
    Period,
    /// Hour of day (UTC): the histogram of §6.1.
    Hour,
    /// Day of week (UTC): the other histogram of §6.1.
    Weekday,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum VolumeDimension {
    Kind,
    Fiat,
    Instance,
    Period,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MarketDimension {
    Fiat,
    Kind,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TimingDimension {
    Fiat,
    Method,
    Kind,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstanceOrPeriod {
    Instance,
    Period,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DisputeDimension {
    Status,
    Initiator,
    Instance,
    Period,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SeriesSplit {
    Instance,
    Kind,
    Fiat,
}

/// Accepts either a unix timestamp or a `YYYY-MM-DD` date, which is read as
/// midnight UTC at the start of that day.
///
/// Both forms produce a unix timestamp; what the window does with it — which
/// end is inclusive — is decided once in [`crate::commands`], not here.
fn parse_timestamp(value: &str) -> Result<i64, String> {
    if let Ok(timestamp) = value.parse::<i64>() {
        if timestamp < 0 {
            return Err(format!(
                "`{value}` is before the unix epoch; expected a unix timestamp or YYYY-MM-DD"
            ));
        }
        return Ok(timestamp);
    }

    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|date| date.and_hms_opt(0, 0, 0).expect("midnight is a valid time"))
        .map(|naive| naive.and_utc().timestamp())
        .map_err(|_| format!("`{value}` is neither a unix timestamp nor a YYYY-MM-DD date"))
}

#[cfg(test)]
mod tests;
