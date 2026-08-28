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

/// The span a series partition covers, in unix seconds, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub from: i64,
    pub until: i64,
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
/// `period` is only meaningful for a series partition and only ever given
/// for one; a window document is relative to the run and has no fixed
/// span to name.
pub fn tags(address: &Address, run: &Run, revision: u32, period: Option<Period>) -> Vec<Tag> {
    let mut tags = vec![
        Tag::single("d", address.to_string()),
        Tag::single("s", run.snapshot_id.clone()),
        Tag::single("t", TOPIC),
        Tag::single("alt", alt(address)),
    ];

    if let Address::Series { resolution, .. } = address {
        tags.push(Tag::single("resolution", resolution.as_str()));
        if let Some(period) = period {
            tags.push(Tag {
                name: "period".to_string(),
                values: vec![rfc3339(period.from), rfc3339(period.until)],
            });
        }
    }

    tags.push(Tag::single("revision", revision.to_string()));
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
            resolution,
            bucket,
            scope,
        } => format!(
            "bestiario {} series, {} buckets, {}{}",
            describe(*report),
            resolution.as_str(),
            partition(*bucket),
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

fn partition(bucket: Bucket) -> String {
    match bucket {
        Bucket::Month { .. } => format!("month {bucket}"),
        Bucket::Year(_) => format!("year {bucket}"),
    }
}

/// A document's content (§6): the run around the answer.
///
/// Field order is part of the format — run first, answer last — and serde
/// keeps declaration order, so the struct *is* the order. The two
/// restatement fields are absent rather than null when there was none: an
/// absent restatement is not a restatement with empty reasons.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Envelope {
    pub schema_version: u32,
    pub snapshot_id: String,
    /// RFC 3339, UTC.
    pub generated_at: String,
    pub revision: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restated_because: Option<String>,
    /// The figures, and only the figures: the part the hash of §5 covers.
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn new(run: &Run, revision: u32, payload: serde_json::Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            snapshot_id: run.snapshot_id.clone(),
            generated_at: rfc3339(run.generated_at),
            revision,
            restated_at: None,
            restated_because: None,
            payload,
        }
    }

    /// The same document, marked as restated at `at` for `because` (§8).
    pub fn restated(self, at: i64, because: &str) -> Self {
        Self {
            restated_at: Some(rfc3339(at)),
            restated_because: Some(because.to_string()),
            ..self
        }
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
