//! The index — `docs/NOSTR-PUBLICATION.md` §5.
//!
//! `d = index`: the first document a client fetches and the only address
//! it has to know a priori. It answers the three questions a client
//! cannot answer for itself.
//!
//! 1. **What exists.** Which partitions were published, at which
//!    resolutions, from when. Without it a client guesses, asks for
//!    months nobody published, and cannot tell "no data" from "not
//!    published".
//! 2. **What changed.** Each entry quotes the *document's own* payload
//!    hash — the figures, never the run around them. Hashing the whole
//!    content would give every closed partition a new hash, a new
//!    revision and a fresh signature on every run, and the skip of §8,
//!    the cache of §10 and the point of `revision` would all be dead
//!    letters.
//! 3. **What is current.** The `snapshot_id` of §7, carried by the
//!    envelope like every other document's.
//!
//! `coverage` states the archive's real extent, both ends. A client MUST
//! NOT render a period outside it as zero (§6.3) — which is what makes an
//! empty extent worth stating rather than omitting: the window documents
//! of an archive that holds nothing are still published, still full of
//! zeros, and this is the only thing that says why.
//!
//! # Growing into shards
//!
//! §5.1 has the index sharding by year once it approaches the size limit
//! of §9.1 — `index:2026`, `index:2027`, with the unqualified `index`
//! listing the hot documents and the shards. That change is additive: a
//! shard is another `Address::Index { year: Some(_) }` over a subset of
//! the same entries, so nothing here has to be re-shaped to allow it and
//! no client has to be told.

use std::collections::BTreeMap;

use serde::Serialize;

use super::address::Address;
use super::document::{Envelope, rfc3339};
use super::snapshot::{Document, Snapshot};

/// Who published a snapshot, as the index names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Publisher {
    pub name: String,
    pub version: String,
}

/// The archive's real extent: `null` at both ends when it holds nothing,
/// which is a fact about the archive and not a missing field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Extent {
    pub first_event_at: Option<String>,
    pub last_event_at: Option<String>,
}

/// The span of published partitions at one resolution, as bucket keys —
/// `2026-02` for a month, `2026` for a year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Available {
    pub from: String,
    pub until: String,
}

/// One published document, as the index lists it.
///
/// `updated_at` is when the payload last changed, not when the document
/// was last published — the same reason the hash is over the payload. A
/// first revision restates nothing, so both restatement fields are absent
/// rather than null (§8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub d: String,
    pub hash: String,
    pub revision: u32,
    /// RFC 3339, UTC.
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restated_because: Option<String>,
}

/// The `payload` of the index document.
///
/// `resolutions` is a `BTreeMap` rather than an insertion-ordered map for
/// the same reason every other payload here serialises deterministically:
/// §8 skips a document whose payload has not changed, and a map that
/// ordered its keys by chance would make the index a new revision on
/// every run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Payload {
    pub publisher: Publisher,
    pub coverage: Extent,
    pub resolutions: BTreeMap<String, Available>,
    pub documents: Vec<Entry>,
}

impl Snapshot {
    /// The index over the documents this snapshot computed.
    ///
    /// The index is not one of them: it is how a client finds the rest,
    /// and an index listing itself would be a hash of a hash of itself.
    pub fn index(&self, publisher: &Publisher) -> Document {
        let payload = Payload {
            publisher: publisher.clone(),
            coverage: Extent {
                first_event_at: self.coverage.earliest().map(rfc3339),
                last_event_at: self.coverage.latest().map(rfc3339),
            },
            resolutions: resolutions(&self.documents),
            documents: self.documents.iter().map(entry).collect(),
        };

        Document {
            address: Address::Index { year: None },
            hash: super::snapshot::hash_of(&payload),
            updated_at: self.run.generated_at,
            envelope: Envelope::first(
                &self.run,
                serde_json::to_value(&payload).expect("plain data"),
            ),
            period: None,
        }
    }
}

/// One document's entry, read entirely off the document: the revision it
/// is published under, when its figures last moved, and — above the first
/// revision — when and why they moved (§8).
///
/// Nothing is recomputed here. A snapshot on its own has no history, and
/// `Snapshot::restated` is what folds the last publication into these
/// fields; an index built from a snapshot that never met its history
/// reports every document as a first revision, which is exactly what a
/// first publication is.
fn entry(document: &Document) -> Entry {
    Entry {
        d: document.address.to_string(),
        hash: document.hash.clone(),
        revision: document.envelope.revision(),
        updated_at: rfc3339(document.updated_at),
        restated_at: document.envelope.restated_at().map(str::to_string),
        restated_because: document.envelope.restated_because().map(str::to_string),
    }
}

/// The resolutions a client may pick from, and the bucket keys each one
/// spans — read off the documents that were actually published rather
/// than off what the archive could have covered, so the index never
/// sends a client after a document that does not exist.
///
/// `from` and `until` are the least and greatest keys as text. Every key
/// at one resolution has the same fixed, zero-padded shape (`2026-02`,
/// `2026`), so ordering them as strings is ordering them as dates.
fn resolutions(documents: &[Document]) -> BTreeMap<String, Available> {
    let mut spans: BTreeMap<String, Available> = BTreeMap::new();

    for document in documents {
        let Address::Series { partition, .. } = &document.address else {
            continue;
        };
        let key = partition.bucket().to_string();
        spans
            .entry(partition.resolution().as_str().to_string())
            .and_modify(|span| {
                span.from = span.from.clone().min(key.clone());
                span.until = span.until.clone().max(key.clone());
            })
            .or_insert_with(|| Available {
                from: key.clone(),
                until: key,
            });
    }

    spans
}

#[cfg(test)]
mod tests;
