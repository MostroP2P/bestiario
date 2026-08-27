//! Pure aggregations over Mostro network data.
//!
//! Responsibility: the metric catalog of `docs/SPEC.md` §6. This crate
//! receives data that the indexer has already loaded and returns
//! serializable results.
//!
//! # The no-I/O invariant
//!
//! This is a **separate crate** rather than a module of `bestiario`, and that
//! is the whole point. `docs/SPEC.md` §8 requires the aggregation layer to
//! stay free of I/O so an HTTP API can reuse it unchanged (phase 5). A module
//! could satisfy that rule only by convention: any file in the crate can
//! reach for `sqlx` the moment it is convenient, and someone has to notice.
//!
//! As its own crate with a four-entry dependency list, most of the rule is
//! enforced by cargo: `sqlx`, `nostr-sdk`, `tokio` and `reqwest` are not in
//! scope, so code that wants them does not compile. Relaxing that means
//! editing a manifest, which is a deliberate act that shows up in review, and
//! `scripts/check-stats-deps.sh` fails the build when the list grows.
//!
//! Cargo does not cover all of it. `std` is in scope regardless of the
//! manifest, and `std::fs`, `std::net` and `std::process` can perform I/O
//! without any dependency at all. `clippy.toml` closes that gap by
//! disallowing those entry points; because clippy resolves paths rather than
//! matching text, `use std::{fs::File, io::Write}` is caught the same as a
//! fully written-out path.
//!
//! Aggregations therefore take plain structs and return plain structs, which
//! is also what makes them testable against a hand-built dataset and a
//! hand-computed expected value.

pub mod activity;
pub mod compare;
pub mod dev_fees;
pub mod disputes;
pub mod instances;
pub mod lifecycle;
pub mod metric;
pub mod percentile;
pub mod summary;
pub mod volume;
pub mod window;

pub use metric::{Metric, MetricKind, Value};
pub use window::Window;
