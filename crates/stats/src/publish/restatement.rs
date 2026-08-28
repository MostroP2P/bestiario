//! Revisions, and the documents a run does not send —
//! `docs/NOSTR-PUBLICATION.md` §8.
//!
//! A closed month is not immutable in practice: a later backfill finds
//! older events, a `rebuild` recomputes projections, a dev fee arrives
//! late. Published figures change, and §8 says that must not be silent.
//! So every document carries a `revision`, and a revision above the first
//! carries when it was restated and why.
//!
//! # What this module decides, and what it cannot
//!
//! A snapshot on its own knows nothing of the one before it — it is a
//! function of the archive and the clock, and that is what makes it
//! reproducible. The comparison against the last publication is therefore
//! a separate step over plain data: what was published under each address
//! ([`Previous`]) meets what this run computed, and the result is the
//! revision each document carries and whether it is sent at all.
//!
//! Reading that history out of a database and writing it back is the
//! binary's; this crate performs no I/O. What is here is the rule, which
//! is the part worth testing as a rule.
//!
//! # Why an unchanged document is not re-sent
//!
//! §8: a document whose payload hashes to what is already published is
//! not re-signed and not sent. Nothing about the answer changed, and a
//! relay does not need a second copy of it. It stays in the index — with
//! the hash, revision and `updated_at` it already had, because
//! "unchanged" is one of the things the index exists to say.
//!
//! The exception is `--republish` (§9.3), which exists precisely to
//! distrust the assumption that the relay already holds something.

use std::collections::{BTreeMap, BTreeSet};

use super::address::Address;
use super::document::{Envelope, Restatement};
use super::snapshot::{Document, Snapshot};
use crate::window::Window;

/// What the last publication concluded about one address.
///
/// The two states of §8 and no third, for the same reason [`Envelope`]
/// has two: a revision above the first *is* a restatement and carries its
/// provenance, and the first revision has nothing to restate. Recorded
/// state can be read back from a database with a null where a value
/// belongs, and a shape that could hold "revision 3, restated for no
/// reason" would publish it as revision 1 without anybody noticing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Previous {
    /// Published once, never restated.
    First {
        /// The hash of the payload that was published — the comparison
        /// that decides everything here.
        hash: String,
        /// Unix seconds: when the figures last changed, not when they
        /// were last looked at.
        updated_at: i64,
    },
    /// Published, and restated at least once.
    Restated {
        hash: String,
        revision: u32,
        updated_at: i64,
        restatement: Restatement,
    },
}

impl Previous {
    /// A record above the first revision; `None` when `revision` is not
    /// above it, which is [`Previous::First`]'s case and not this one.
    pub fn restated(
        hash: impl Into<String>,
        revision: u32,
        updated_at: i64,
        restatement: Restatement,
    ) -> Option<Self> {
        (revision > 1).then(|| Self::Restated {
            hash: hash.into(),
            revision,
            updated_at,
            restatement,
        })
    }

    pub fn hash(&self) -> &str {
        match self {
            Self::First { hash, .. } | Self::Restated { hash, .. } => hash,
        }
    }

    pub fn revision(&self) -> u32 {
        match self {
            Self::First { .. } => 1,
            Self::Restated { revision, .. } => *revision,
        }
    }

    pub fn updated_at(&self) -> i64 {
        match self {
            Self::First { updated_at, .. } | Self::Restated { updated_at, .. } => *updated_at,
        }
    }

    pub fn restatement(&self) -> Option<&Restatement> {
        match self {
            Self::First { .. } => None,
            Self::Restated { restatement, .. } => Some(restatement),
        }
    }
}

/// Why the figures moved (§8).
///
/// An enumeration rather than a string because it is a closed set that a
/// client reads: a fifth reason nobody agreed on would reach a rendering
/// that has no case for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Because {
    /// A backfill reached further back than any before it, so periods
    /// that were partly covered are now covered further.
    Backfill,
    /// The projections were recomputed from the stored versions.
    Rebuild,
    /// The document format changed under the same figures.
    Schema,
    /// Anything an operator had to state for themselves.
    Correction,
}

impl Because {
    /// Which of the four reasons the archive itself says it was.
    ///
    /// The daemon is not told why the figures moved: `publish` does not
    /// know whether a `backfill` or a `rebuild` ran before it. But the
    /// archive records enough to tell the three that matter apart, and a
    /// reason read off the archive is a fact rather than whatever the
    /// operator typed — which is the difference between a field a client
    /// can act on and one it has to discount.
    ///
    /// - The schema published under has changed: `schema`. It is checked
    ///   first because a schema bump moves every payload at once, and any
    ///   backfill in the same run is the smaller half of the story.
    /// - The archive now reaches further back than the last run's did, or
    ///   simply holds more events than it did: `backfill`. Coverage alone
    ///   is not enough — a walk that finds events *inside* a range
    ///   already covered moves no floor and moves plenty of figures —
    ///   and neither is the count, since a deeper walk that finds nothing
    ///   still deepens what the archive can speak for.
    /// - Otherwise the figures moved with the same events underneath
    ///   them: `rebuild`.
    ///
    /// `Correction` is never inferred. It is the reason an operator has
    /// to state for themselves, and nothing in the archive distinguishes
    /// it from a rebuild.
    pub fn inferred(previous: Read, current: Read) -> Self {
        if previous.schema_version != super::document::SCHEMA_VERSION {
            return Self::Schema;
        }

        // An archive that held nothing and now holds something has
        // reached back from nowhere, which is the extreme case of the
        // same thing rather than a case of its own.
        let deepened = match (previous.covered_from, current.covered_from) {
            (Some(before), Some(now)) => now < before,
            (None, Some(_)) => true,
            (_, None) => false,
        };

        if deepened || current.events > previous.events {
            Self::Backfill
        } else {
            Self::Rebuild
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::Rebuild => "rebuild",
            Self::Schema => "schema",
            Self::Correction => "correction",
        }
    }
}

/// What a run sends beyond what changed (§9.3).
///
/// `--republish` exists to put documents on a relay that does not have
/// them, so "the relay already has this" is precisely the assumption it
/// distrusts. The documents a pruned relay is missing are overwhelmingly
/// the ones whose figures have not moved in months, and a run honouring
/// the skip of §8 would send it nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Republish {
    /// An ordinary run: a payload already published is not re-sent.
    No,
    /// Every document, whatever its hash.
    All,
    /// Every series partition this range touches, whatever its hash —
    /// plus whatever an ordinary run would have sent anyway.
    ///
    /// The second half is not a convenience. The index names every
    /// document with the hash of the payload that belongs to it, so a
    /// range republication that withheld a *changed* document outside its
    /// range would publish an index pointing at something no relay holds
    /// (§7).
    Range(Window),
}

impl Republish {
    /// Whether a document whose payload is already published is sent
    /// anyway.
    fn sends_unchanged(self, document: &Document) -> bool {
        match self {
            Self::No => false,
            Self::All => true,
            // A window document is relative to the run and covers no
            // fixed span, so no range names it. Recovering a range means
            // recovering its partitions.
            Self::Range(range) => match (&document.address, document.period) {
                (Address::Series { .. }, Some(period)) => {
                    period.from < range.until && range.from < period.until
                }
                _ => false,
            },
        }
    }
}

/// What one run read out of the archive, as far as telling the reasons of
/// §8 apart requires.
///
/// A struct rather than three loose arguments because two of them are the
/// same shape — a previous reading and a current one — and a caller that
/// swapped them would report every backfill as a rebuild and every
/// rebuild as nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
    /// The schema the run published under. On the current reading this is
    /// always the crate's own; on the previous one it is what was
    /// recorded, which is the comparison.
    pub schema_version: u32,
    /// The oldest event the archive could speak for; `None` when it held
    /// nothing.
    pub covered_from: Option<i64>,
    /// How many events it held.
    pub events: u64,
}

/// A snapshot placed against what was last published.
#[derive(Debug, Clone, PartialEq)]
pub struct Restated {
    /// Every document of the run, each carrying the revision it is
    /// published under and the clock its figures last moved at. The index
    /// is built from this, so it lists the unchanged ones too.
    pub snapshot: Snapshot,
    /// The addresses this run does not send: their payload is already on
    /// the relay, and no republication asked for them anyway. Never a
    /// reason to leave one out of the index — "unchanged" is one of the
    /// things the index exists to say.
    pub not_sent: BTreeSet<String>,
    /// What this run leaves behind for the next one to compare against —
    /// the same shape it was handed.
    ///
    /// Produced here rather than derived by the caller from the documents
    /// because the derivation *is* the rule: a caller reconstructing
    /// "which revision did this end up at" from an envelope would be
    /// reimplementing [`Snapshot::restated`] in order to record its
    /// result.
    pub state: BTreeMap<String, Previous>,
}

impl Snapshot {
    /// This snapshot, with each document's revision decided against what
    /// was last published under its address.
    ///
    /// `because` is the reason attached to every document whose figures
    /// moved in this run. One reason per run rather than one per
    /// document: the four of §8 are properties of what happened to the
    /// archive between two runs, and a document cannot have been restated
    /// for a reason the run did not have.
    ///
    /// `republish` says which documents are sent beyond the ones that
    /// changed, without touching any revision: re-signing an unchanged
    /// payload is not a restatement (§9.3).
    pub fn restated(
        &self,
        previous: &BTreeMap<String, Previous>,
        because: Because,
        republish: Republish,
    ) -> Restated {
        let mut not_sent = BTreeSet::new();
        let mut state = BTreeMap::new();
        let mut documents = Vec::with_capacity(self.documents.len());

        for document in &self.documents {
            let address = document.address.to_string();
            let (restated, is_unchanged) = self.against(document, previous.get(&address), because);
            if is_unchanged && !republish.sends_unchanged(&restated) {
                not_sent.insert(address.clone());
            }
            state.insert(address, recorded(&restated));
            documents.push(restated);
        }

        Restated {
            snapshot: Snapshot {
                run: self.run.clone(),
                coverage: self.coverage,
                documents,
            },
            not_sent,
            state,
        }
    }

    /// One document against its own history. The boolean is whether the
    /// payload is the one already published.
    fn against(
        &self,
        document: &Document,
        previous: Option<&Previous>,
        because: Because,
    ) -> (Document, bool) {
        let payload = document.envelope.payload().clone();

        let Some(previous) = previous else {
            // Never published under this address: revision 1, restating
            // nothing, and its figures are as new as the run.
            return (
                Document {
                    envelope: Envelope::first(&self.run, payload),
                    updated_at: self.run.generated_at,
                    ..document.clone()
                },
                false,
            );
        };

        if previous.hash() == document.hash {
            // The figures did not move, so neither does anything that
            // records when they last did. The envelope is rebuilt only so
            // that the index can read the revision off it; the event
            // carrying it is the one already on the relay.
            return (
                Document {
                    envelope: envelope(
                        &self.run,
                        previous.revision(),
                        previous.restatement().cloned(),
                        payload,
                    ),
                    updated_at: previous.updated_at(),
                    ..document.clone()
                },
                true,
            );
        }

        let revision = previous.revision().saturating_add(1);
        (
            Document {
                envelope: envelope(
                    &self.run,
                    revision,
                    Some(Restatement {
                        at: self.run.generated_at,
                        because: because.as_str().to_string(),
                    }),
                    payload,
                ),
                updated_at: self.run.generated_at,
                ..document.clone()
            },
            false,
        )
    }
}

/// What a document leaves behind: the two fields the next run compares
/// against, and the revision it reached.
///
/// A restatement here is the document's own, whether it acquired it in
/// this run or carried it in from the last one — which is what makes the
/// record idempotent: writing back an unchanged document writes what was
/// already there.
pub fn recorded(document: &Document) -> Previous {
    document
        .envelope
        .restatement()
        .and_then(|restatement| {
            Previous::restated(
                document.hash.clone(),
                document.envelope.revision(),
                document.updated_at,
                restatement,
            )
        })
        .unwrap_or_else(|| Previous::First {
            hash: document.hash.clone(),
            updated_at: document.updated_at,
        })
}

/// An envelope at `revision`, restated or not.
///
/// The `Option` mirrors [`Previous`]'s two states exactly: a restatement
/// implies a revision above the first, and its absence implies the first.
/// [`Envelope::restated`] refuses a revision of 1, and by construction no
/// caller here reaches that refusal.
fn envelope(
    run: &super::document::Run,
    revision: u32,
    restatement: Option<Restatement>,
    payload: serde_json::Value,
) -> Envelope {
    restatement
        .and_then(|restatement| Envelope::restated(run, revision, restatement, payload.clone()))
        .unwrap_or_else(|| Envelope::first(run, payload))
}

#[cfg(test)]
mod tests;
