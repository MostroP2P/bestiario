//! The ingestion pipeline: relay event in, persisted rows out.
//!
//! Responsibility: the seven steps of `docs/SPEC.md` §8.1 — verify the
//! signature, apply the instance and network filters, deduplicate, parse by
//! kind, persist the version and refresh the projection in a single
//! transaction, and advance the sync cursor.

pub mod parse;
