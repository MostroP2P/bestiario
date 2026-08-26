//! bestiario — an indexer for the public Nostr events published by Mostro
//! instances, and the statistics derived from them.
//!
//! The crate is split into a library and a thin binary so that integration
//! tests under `tests/` can drive the same code paths the CLI does.
//!
//! # Layering
//!
//! ```text
//! commands/  ← CLI entry points, one per subcommand
//!   report/  ← rendering: tables and the JSON envelope
//!   stats    ← pure aggregations (the bestiario-stats crate)
//!   db/      ← SQLite pool, migrations, one repository per table
//!   ingest/  ← the pipeline: verify, dedup, parse, persist
//!   nostr/   ← relay connections and filter construction
//!   config/  ← settings.toml
//! ```
//!
//! Dependencies point downwards only. See `docs/SPEC.md` §8.
//!
//! The aggregation layer lives in the separate `bestiario-stats` crate, whose
//! restricted dependency list is what enforces its no-I/O invariant. It is
//! re-exported here as [`stats`] so callers keep saying `bestiario::stats::…`.

pub mod cli;
pub mod commands;
pub mod config;
pub mod db;
pub mod ingest;
pub mod logging;
pub mod nostr;
pub mod report;

pub use bestiario_stats as stats;
