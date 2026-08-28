//! The document model of `docs/NOSTR-PUBLICATION.md` — what a published
//! snapshot is made of, before anything is computed, signed or sent.
//!
//! Two halves. [`address`] is the `d` grammar of §3: the name a client
//! constructs to fetch a document, parsed and rendered as inverses so a
//! typo is a miss and never a fuzzy match. [`document`] is the rest of the
//! shape — the kind, the tag set of §11 and the envelope of §6 that puts
//! the run around the figures.
//!
//! Pure functions over plain data, like the rest of this crate: the
//! grammar is testable as a grammar, without a relay or an archive in the
//! way. Computing a snapshot, hashing its payloads and publishing it are
//! the binary's, in the roadmap rows that follow this one.

pub mod address;
pub mod document;
pub mod index;
pub mod snapshot;

pub use address::Address;
pub use document::{Envelope, KIND, Run, SCHEMA_VERSION, Tag};
