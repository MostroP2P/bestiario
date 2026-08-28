//! The size gate — `docs/NOSTR-PUBLICATION.md` §9.1.
//!
//! A relay advertises `limitation.max_content_length` in its NIP-11
//! document, and the publisher carries a ceiling of its own
//! (`[publish].max_content_bytes`, 64 KiB by default and conservative on
//! purpose). Every document is checked against the smallest of them
//! *before* anything is signed, so that a document too large to publish
//! is an error naming the document rather than a silent rejection by one
//! relay and an accepted event on another — which would leave a snapshot
//! whose index names a document that is not there.
//!
//! Reading the NIP-11 documents is the binary's; weighing bytes against a
//! number is not, and it lives here with the documents it weighs.

use super::address::Address;
use super::index::Index;
use super::snapshot::Document;

/// The publisher's own ceiling when the configuration does not set one.
/// Conservative: relays that advertise nothing are common, and a limit
/// nobody stated is not a limit that does not exist.
pub const DEFAULT_MAX_CONTENT_BYTES: usize = 64 * 1024;

/// The largest `content` any document may carry, and what set it.
///
/// The configured ceiling always applies. A relay lowers it, never raises
/// it: a relay that would accept a megabyte does not make a document the
/// publisher considers too large acceptable, and the smallest ceiling is
/// the only one under which every configured relay stores the same
/// snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ceiling {
    bytes: usize,
    relay: Option<String>,
}

impl Ceiling {
    /// The publisher's own ceiling, before any relay has been asked.
    pub fn configured(bytes: usize) -> Self {
        Self { bytes, relay: None }
    }

    /// The ceiling with `relay`'s advertised limit folded in, which
    /// changes it only when the relay's is smaller.
    #[must_use]
    pub fn and_relay(self, relay: &str, advertised: usize) -> Self {
        if advertised < self.bytes {
            Self {
                bytes: advertised,
                relay: Some(relay.to_string()),
            }
        } else {
            self
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The relay whose advertised limit is in force, or `None` when the
    /// configured ceiling is the binding one.
    pub fn relay(&self) -> Option<&str> {
        self.relay.as_deref()
    }
}

/// One document weighed: what a `--dry-run` prints and what the gate
/// compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    pub address: Address,
    pub bytes: usize,
    /// The payload hash of §5, carried alongside so a review of a
    /// snapshot answers "how big" and "did it change" from one listing.
    ///
    /// `None` for the index, the one document nothing hashes (§6): it is
    /// what the hashes are *in*, and a review has nothing to compare it
    /// against between runs. Its size still counts.
    pub hash: Option<String>,
}

/// Every document of a snapshot, weighed in the order it was computed.
pub fn measure(documents: &[Document]) -> Vec<Measured> {
    documents
        .iter()
        .map(|document| Measured {
            address: document.address.clone(),
            bytes: document.content().len(),
            hash: Some(document.hash.clone()),
        })
        .collect()
}

/// The index, weighed the same way. Separate because the index is not a
/// [`Document`] — §6 exempts it from the envelope every other document
/// carries — but §9.1 weighs it all the same, and §5.1 shards it by year
/// when it stops fitting.
pub fn measure_index(index: &Index) -> Measured {
    Measured {
        address: index.address(),
        bytes: index.content().len(),
        hash: None,
    }
}

/// The documents that do not fit under `ceiling`.
///
/// A list rather than the first one: an operator who has to shrink a
/// snapshot wants to know how much of it is over, not to discover the
/// next one on the next run.
pub fn over<'a>(measured: &'a [Measured], ceiling: &Ceiling) -> Vec<&'a Measured> {
    measured
        .iter()
        .filter(|document| document.bytes > ceiling.bytes())
        .collect()
}

#[cfg(test)]
mod tests;
