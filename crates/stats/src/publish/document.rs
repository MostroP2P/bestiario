//! What a published document is made of, apart from its figures — the
//! event kind of `docs/NOSTR-PUBLICATION.md` §2, the tag set of §11 and the
//! envelope of §6.
//!
//! Nothing here signs, serialises for the wire or talks to a relay: those
//! are the binary's, and this crate performs no I/O. What it owns is the
//! *shape* — which tags a document carries and what they say, and which
//! fields describe the run as opposed to the answer — so that the shape is
//! testable as a shape, and a client author can read it without reading a
//! relay client.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::address::{Address, Bucket, Report, Scope, Window};
use crate::window::Window as Span;

/// The addressable kind every document is published under (§2): the one
/// candidate a reader remembers without consulting the spec.
pub const KIND: u16 = 30666;

/// The format version every document carries. Bumped when the `d` grammar
/// of §3 or a payload shape of §6 changes in a way a client has to know.
pub const SCHEMA_VERSION: u32 = 1;

/// The `t` tag every document carries, for discovery.
pub const TOPIC: &str = "bestiario";

/// One publication run: what every document of a snapshot shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Monotonic and unique per run (§7); the `s` tag.
    pub snapshot_id: String,
    /// Unix seconds.
    pub generated_at: i64,
}

/// One Nostr tag: a name and one or more values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub values: Vec<String>,
}

impl Tag {
    fn single(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            values: vec![value.into()],
        }
    }

    /// The first value — every tag here has one, and `period` has two.
    pub fn value(&self) -> &str {
        self.values.first().map_or("", String::as_str)
    }
}

/// The tags of §11 for one document, in the order the table lists them.
///
/// A series partition names its `resolution` and `period`, both read off
/// the address rather than passed alongside it: the address already says
/// which month or year the partition covers, and a caller that could
/// forget the period would eventually sign a document without one. A
/// window document is relative to the run and has no fixed span to name.
///
/// `revision` is `None` for the index alone. The tag mirrors the content
/// field of §8, and the index has none: it carries no payload to restate
/// and is republished on every run by definition (§6). A tag saying `1`
/// every night would claim a first publication that never happened.
pub fn tags(address: &Address, run: &Run, revision: Option<u32>) -> Vec<Tag> {
    let mut tags = vec![
        Tag::single("d", address.to_string()),
        Tag::single("s", run.snapshot_id.clone()),
        Tag::single("t", TOPIC),
        Tag::single("alt", alt(address)),
    ];

    if let Address::Series { partition, .. } = address {
        let Span { from, until } = partition.window();
        tags.push(Tag::single("resolution", partition.resolution().as_str()));
        tags.push(Tag {
            name: "period".to_string(),
            values: vec![rfc3339(from), rfc3339(until)],
        });
    }

    if let Some(revision) = revision {
        tags.push(Tag::single("revision", revision.to_string()));
    }
    tags.push(Tag::single("schema_version", SCHEMA_VERSION.to_string()));
    tags
}

/// The NIP-31 `alt` text: one sentence a client that does not know the
/// kind can show instead of JSON.
fn alt(address: &Address) -> String {
    let scoped = |scope: &Option<Scope>| match scope {
        None => String::new(),
        Some(Scope::Instance(pubkey)) => format!(" for instance {pubkey}"),
        Some(Scope::Network(network)) => format!(" on {network}"),
    };

    match address {
        Address::Index { year: None } => {
            "bestiario index: the documents of the current snapshot".to_string()
        }
        Address::Index { year: Some(year) } => {
            format!("bestiario index for {year}: the documents of that year")
        }
        Address::Window {
            report,
            window,
            scope,
        } => format!(
            "bestiario {} report over the last {}{}",
            describe(*report),
            span(*window),
            scoped(scope)
        ),
        Address::Series {
            report,
            partition,
            scope,
        } => format!(
            "bestiario {} series, {} buckets, {}{}",
            describe(*report),
            partition.resolution().as_str(),
            spanned(partition.bucket()),
            scoped(scope)
        ),
    }
}

fn describe(report: Report) -> &'static str {
    match report {
        Report::Summary => "network summary",
        Report::Orders => "orders",
        Report::Volume => "volume",
        Report::Market => "market structure",
        Report::Disputes => "disputes",
        Report::DevFees => "dev fees",
        Report::Instances => "instances",
        Report::Compare => "instance comparison",
    }
}

fn span(window: Window) -> &'static str {
    match window {
        Window::Hours24 => "24 hours",
        Window::Days7 => "7 days",
        Window::Days30 => "30 days",
        Window::Days90 => "90 days",
        Window::All => "whole archive",
    }
}

fn spanned(bucket: Bucket) -> String {
    match bucket {
        Bucket::Month { .. } => format!("month {bucket}"),
        Bucket::Year(_) => format!("year {bucket}"),
    }
}

/// Why a document's figures moved (§8): what every revision above the
/// first carries, and what the first cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restatement {
    /// Unix seconds.
    pub at: i64,
    pub because: String,
}

/// A document's content (§6): the run around the answer.
///
/// Field order is part of the format — run first, answer last — and serde
/// keeps declaration order, so the struct *is* the order. The two
/// restatement fields are absent rather than null when there was none: an
/// absent restatement is not a restatement with empty reasons.
///
/// The fields are read-only from outside because two of them move
/// together: a revision above the first *is* a restatement and carries
/// its provenance (§8), and the first revision has nothing to restate.
/// [`Envelope::first`] and [`Envelope::restated`] are the two states, and
/// there is no third.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Envelope {
    schema_version: u32,
    snapshot_id: String,
    /// RFC 3339, UTC.
    generated_at: String,
    revision: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    restated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    restated_because: Option<String>,
    /// The figures, and only the figures: the part the hash of §5 covers.
    payload: serde_json::Value,
}

impl Envelope {
    /// The first publication of a document: revision 1, nothing restated.
    pub fn first(run: &Run, payload: serde_json::Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            snapshot_id: run.snapshot_id.clone(),
            generated_at: rfc3339(run.generated_at),
            revision: 1,
            restated_at: None,
            restated_because: None,
            payload,
        }
    }

    /// A later revision, which is a restatement and says why (§8).
    /// `revision` is the number this publication carries, and must be
    /// above the first: `None` when it is not.
    pub fn restated(
        run: &Run,
        revision: u32,
        restatement: Restatement,
        payload: serde_json::Value,
    ) -> Option<Self> {
        (revision > 1).then(|| Self {
            revision,
            restated_at: Some(rfc3339(restatement.at)),
            restated_because: Some(restatement.because),
            ..Self::first(run, payload)
        })
    }

    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub fn generated_at(&self) -> &str {
        &self.generated_at
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn payload(&self) -> &serde_json::Value {
        &self.payload
    }
}

/// Unix seconds as RFC 3339 in UTC, the way every timestamp in a
/// document is written.
pub fn rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|at| at.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

#[cfg(test)]
mod tests;
