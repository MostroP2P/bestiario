//! Read-side loaders that feed the aggregation crate.
//!
//! Responsibility: turning rows into the plain structs `bestiario::stats`
//! computes over, and nothing else. A repository owns one table; a loader
//! reads across several and produces exactly the shape one metric family
//! needs, so the stats crate — which cannot see SQLite — is handed data
//! rather than a connection (`docs/SPEC.md` §8).
//!
//! Loaders filter on what the database indexes (instance, network) and leave
//! the time window to the aggregation, which needs the previous period and
//! the orders still open *now* as well as the window itself.

pub mod activity;
pub mod dev_fees;
pub mod disputes;
pub mod instances;
pub mod rates;

use sqlx::{QueryBuilder, Sqlite};

use crate::network::Network;

/// What every loader narrows its read to.
///
/// An empty `networks` means no network filter, in the same way an empty
/// author list means any author in the relay filters. Configuration never
/// produces one — `networks` is validated non-empty — so the case exists for
/// callers, not for users.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    /// Lowercase hex; `None` for every instance.
    pub pubkey: Option<String>,
    pub networks: Vec<Network>,
}

impl Scope {
    /// Appends the `AND …` clauses of this scope to `query`, against the
    /// table aliased `alias`, which must have `pubkey` and `network`
    /// columns. The query must already be inside a `WHERE`.
    pub(crate) fn apply(&self, query: &mut QueryBuilder<Sqlite>, alias: &str) {
        self.apply_instance(query, alias);
        if !self.networks.is_empty() {
            query.push(format!(" AND {alias}.network IN ("));
            let mut networks = query.separated(", ");
            for network in &self.networks {
                networks.push_bind(network.as_str());
            }
            query.push(")");
        }
    }
}

impl Scope {
    /// The instance half of [`apply`](Self::apply) alone, for a table with
    /// no `network` column. Disputes (kind 38386) carry no `network` tag,
    /// so the network filter cannot reach them and is not pretended to.
    pub(crate) fn apply_instance(&self, query: &mut QueryBuilder<Sqlite>, alias: &str) {
        if let Some(pubkey) = &self.pubkey {
            query
                .push(format!(" AND {alias}.pubkey = "))
                .push_bind(pubkey.clone());
        }
    }
}

/// How many leading hex characters identify an instance in a label.
///
/// Eight is what people quote — `82fa8cb9…` — and is comfortably unique
/// across a network of a few dozen pubkeys.
const SHORT_PUBKEY_LEN: usize = 8;

/// How a report names an instance (`docs/SPEC.md` §3): `name (short
/// pubkey)` when it publishes a name, the bare pubkey otherwise.
///
/// The pubkey is always part of the label, because a name is not an
/// identity: two instances can publish the same one, and a slice keyed by
/// name alone would merge them into a bucket that belongs to neither.
pub fn instance_label(pubkey: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => {
            let short: String = pubkey.chars().take(SHORT_PUBKEY_LEN).collect();
            format!("{name} ({short})")
        }
        None => pubkey.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_instance_is_labelled_by_name_and_short_pubkey() {
        assert_eq!(
            instance_label("82fa8cb978b43c79b2156585bac2c011", Some("Alpha")),
            "Alpha (82fa8cb9)"
        );
    }

    #[test]
    fn a_nameless_instance_is_labelled_by_its_whole_pubkey() {
        assert_eq!(
            instance_label("82fa8cb978b43c79b2156585bac2c011", None),
            "82fa8cb978b43c79b2156585bac2c011"
        );
    }
}
