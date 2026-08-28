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
//! 3. **What is current.** The `snapshot_id` of §7, named at the top
//!    level of the index itself.
//!
//! # The one document with no envelope
//!
//! §6 splits every other document into the run and the `payload` it
//! wraps, and hashes only the payload. The index is the stated exception:
//! nothing hashes it — it is what the hashes are *in* — and it is
//! republished on every run by definition. So `publisher`, `coverage`,
//! `resolutions` and `documents` sit at the top level beside
//! `schema_version`, `snapshot_id` and `generated_at`, with no `payload`
//! to nest them under and no `revision` to count. §10 has a client read
//! `snapshot_id`, `coverage` and `resolutions` straight off the index;
//! wrapping them a level deeper would put them where no conforming client
//! looks.
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
use super::document::{SCHEMA_VERSION, rfc3339};
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

/// The index document (§5), whole.
///
/// The one document with no envelope/payload split: nothing hashes the
/// index — it is what the hashes are *in* — and it is republished on
/// every run by definition, since naming the current snapshot is its
/// whole job. So there is no `payload` to hash and no `revision` to
/// count, and `publisher`, `coverage`, `resolutions` and `documents` sit
/// at the top level beside the run's own fields, which is where §10 has a
/// client read them.
///
/// Field order is part of the format — the run first, the answer after —
/// and serde keeps declaration order, so the struct *is* the order.
///
/// `resolutions` is a `BTreeMap` rather than an insertion-ordered map for
/// the same reason every other document here serialises deterministically:
/// §8 skips a document whose figures have not changed, and a map that
/// ordered its keys by chance would give the index new bytes, and clients
/// a new event to re-verify, on every run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Index {
    pub schema_version: u32,
    pub snapshot_id: String,
    /// RFC 3339, UTC.
    pub generated_at: String,
    pub publisher: Publisher,
    pub coverage: Extent,
    pub resolutions: BTreeMap<String, Available>,
    pub documents: Vec<Entry>,
}

impl Index {
    /// The `d` an index is published under (§3) — the only address a
    /// client has to know a priori.
    pub fn address(&self) -> Address {
        Address::Index { year: None }
    }
}

impl Snapshot {
    /// The index over the documents this snapshot computed.
    ///
    /// The index is not one of them: it is how a client finds the rest,
    /// and an index listing itself would be a hash of a hash of itself.
    pub fn index(&self, publisher: &Publisher) -> Index {
        Index {
            schema_version: SCHEMA_VERSION,
            snapshot_id: self.run.snapshot_id.clone(),
            generated_at: rfc3339(self.run.generated_at),
            publisher: publisher.clone(),
            coverage: Extent {
                first_event_at: self.coverage.earliest().map(rfc3339),
                last_event_at: self.coverage.latest().map(rfc3339),
            },
            resolutions: resolutions(&self.documents),
            documents: self.documents.iter().map(entry).collect(),
        }
    }
}

/// One document's entry. `updated_at` is the run's clock because a
/// snapshot on its own knows nothing of the one before it; §8 is what
/// gives an unchanged payload back the clock it already had.
fn entry(document: &Document) -> Entry {
    Entry {
        d: document.address.to_string(),
        hash: document.hash.clone(),
        revision: document.envelope.revision(),
        updated_at: document.envelope.generated_at().to_string(),
        restated_at: None,
        restated_because: None,
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
