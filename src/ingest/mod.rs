//! The ingestion pipeline: relay event in, persisted rows out.
//!
//! Responsibility: the eight steps of `docs/SPEC.md` §8.1 — verify the
//! signature, apply the instance and network filters, deduplicate, parse by
//! kind, persist the version and refresh the projection in a single
//! transaction, and advance the sync cursor. [`pipeline`] is where they live;
//! [`parse`] holds the per-kind tag readers it calls.

pub mod parse;
pub mod pipeline;

pub use pipeline::{IngestOutcome, Pipeline, Policy, Rejection};
