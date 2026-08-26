//! Everything that talks to relays.
//!
//! Responsibility: connect to the configured relays, build the subscription
//! filters, and hand raw events to `ingest`. This module knows nothing about
//! SQLite and nothing about what the events mean — parsing lives in
//! `ingest::parse`. See `docs/SPEC.md` §8.2.

pub mod client;
pub mod filters;
