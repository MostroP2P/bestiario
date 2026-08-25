//! SQLite persistence: connection pool, migrations and repositories.
//!
//! Responsibility: all knowledge of the schema in `docs/SPEC.md` §4. Every
//! write is idempotent — the same event ingested twice must leave the
//! database in the same state as ingesting it once.

pub mod repo;
