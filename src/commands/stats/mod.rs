//! `bestiario stats <family>`: one metric family, freely sliced
//! (`docs/SPEC.md` §10).
//!
//! Each family follows the same three steps, and every submodule is that
//! sequence for one family: resolve the window and the scope from the global
//! flags, load the plain structs the aggregation needs, hand the metrics to
//! the report layer. The families share nothing else, which is why there is
//! no trait: a submodule per family with its own `run` is the whole of it.

pub mod orders;

#[cfg(test)]
mod tests;

use anyhow::Result;

use crate::commands::Context;
use crate::commands::range::{InstanceFilter, Range};
use crate::db::load::Scope;
use crate::db::repo::instances;
use crate::network::Network;

/// What every stats command resolves from the global flags before it loads
/// anything: the window, and which instances and networks it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub range: Range,
    pub scope: Scope,
}

impl Query {
    /// The window and scope the invocation asks for.
    ///
    /// `--network` overrides the configured list; without it, a report
    /// covers every configured network, which is also everything the
    /// indexer admitted. The instance name is resolved against the bestiary
    /// here, so a report for an unknown instance fails before it reads
    /// anything.
    pub async fn resolve(context: &Context<'_>, now: i64) -> Result<Self> {
        let cli = context.cli;
        let range = Range::resolve(cli.from, cli.until, now)?;

        let known: Vec<_> = instances::all(context.pool)
            .await?
            .into_iter()
            .map(|instance| (instance.pubkey, instance.name))
            .collect();
        let pubkey = match InstanceFilter::resolve(cli.instance.as_deref(), &known)? {
            InstanceFilter::All => None,
            InstanceFilter::One { pubkey } => Some(pubkey),
        };

        let networks: Vec<Network> = match cli.network {
            Some(network) => vec![network],
            None => context.settings.indexer.networks.clone(),
        };

        Ok(Self {
            range,
            scope: Scope { pubkey, networks },
        })
    }
}
