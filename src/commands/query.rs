//! The window and the scope every report resolves from the global flags.
//!
//! Resolved once, here, so that the questions the flags raise — which
//! instances, which networks, what a name means — are answered the same way
//! by every command, and so that a report for an unknown instance fails
//! before anything is read.

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
    /// Whether `--network` narrowed the scope below the configured list.
    ///
    /// Dispute events carry no network tag, so a dispute figure cannot be
    /// narrowed the same way; the views that combine families report those
    /// figures as missing when this is set, rather than as a network-wide
    /// number under a network-scoped heading.
    pub network_narrowed: bool,
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
        Self::resolve_for(context, context.cli.instance.as_deref(), now).await
    }

    /// [`resolve`](Self::resolve) with the instance named by `instance`
    /// rather than by `--instance` — for `bestiario instance <PUBKEY|NAME>`,
    /// whose argument is the instance.
    pub async fn resolve_for(
        context: &Context<'_>,
        instance: Option<&str>,
        now: i64,
    ) -> Result<Self> {
        let cli = context.cli;
        let range = Range::resolve(cli.from, cli.until, now)?;

        let known: Vec<_> = instances::all(context.pool)
            .await?
            .into_iter()
            .map(|instance| (instance.pubkey, instance.name))
            .collect();
        let pubkey = match InstanceFilter::resolve(instance, &known)? {
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
            network_narrowed: cli.network.is_some(),
        })
    }
}

#[cfg(test)]
mod tests;
