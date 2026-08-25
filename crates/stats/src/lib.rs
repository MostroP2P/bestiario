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
//! As its own crate with a four-entry dependency list, the rule is enforced
//! by cargo — `sqlx`, `nostr-sdk`, `tokio` and `reqwest` are not in scope, so
//! code that wants them does not compile. Relaxing the invariant now means
//! editing a manifest, which is a deliberate act that shows up in review, and
//! `scripts/check-stats-deps.sh` fails the build when the list grows.
//!
//! Aggregations therefore take plain structs and return plain structs, which
//! is also what makes them testable against a hand-built dataset and a
//! hand-computed expected value.
