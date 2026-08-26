//! One repository per table.
//!
//! Each repository owns the queries for its table and nothing else. Cross-table
//! consistency (a version plus its projection) is the caller's job, inside a
//! transaction.
//!
//! Every function takes an `impl Executor` rather than a `&SqlitePool`, so the
//! same call works against the pool for a one-off write and against a
//! transaction when the pipeline needs several tables to move together
//! (`docs/SPEC.md` §8.1 step 7).

pub mod events;
pub mod instances;
