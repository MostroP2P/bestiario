//! `bestiario stats <family>`: one metric family, freely sliced
//! (`docs/SPEC.md` §10).
//!
//! Each family follows the same three steps, and every submodule is that
//! sequence for one family: resolve the window and the scope from the global
//! flags ([`crate::commands::query::Query`]), load the plain structs the
//! aggregation needs, hand the metrics to the report layer. The families
//! share nothing else, which is why there is no trait: a submodule per
//! family with its own `run` is the whole of it.

pub mod dev_fees;
pub mod disputes;
pub mod orders;
pub mod volume;
