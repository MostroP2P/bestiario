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
