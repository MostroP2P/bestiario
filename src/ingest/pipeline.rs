//! The eight steps of `docs/SPEC.md` §8.1, in the order the spec states them.
//!
//! One event in, one [`IngestOutcome`] out. Everything that decides whether an
//! event is wanted lives here rather than in the parsers or the repositories:
//! a parser says whether an event is *well formed*, a repository says how a
//! value is *stored*, and this module is the only place that says whether an
//! event is *ours*.
//!
//! # Where the archive begins
//!
//! Steps 2 to 5 run before the event is written anywhere, so an event this
//! indexer does not want leaves no trace at all. From step 6 on the event is
//! in `events.raw_json` and stays there — including when the parser rejects
//! it, which is what makes a rejection cost one reprocessing run rather than
//! a re-fetch from relays that may no longer hold the event.
//!
//! # One transaction per accepted event
//!
//! For an event that parses, steps 6, 7 and 8 commit together. The archive row
//! is the dedup gate, so writing it on its own would make any later failure
//! permanent: the raw event would be stored, its version and projection would
//! not, and every retry would read the archive row and answer `Duplicate`
//! without ever repairing it. Committing the three together means an
//! interrupted ingest leaves nothing behind and the next attempt starts over.
//!
//! A parser rejection is the one write that stands alone, because there is
//! nothing to pair it with: the event is archived and that is the whole of it.

use std::collections::HashSet;

use nostr_sdk::prelude::Event;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::config::IndexerSettings;
use crate::db::repo;
use crate::ingest::parse::{self, MOSTRO, ParseError};
use crate::network::Network;

/// The kinds that carry a `y` tag and so pass the platform filter of
/// `docs/SPEC.md` §8.1 step 4.
const PLATFORM_TAGGED: [u16; 4] = [
    parse::order::KIND,
    parse::dev_fee::KIND,
    parse::dispute::KIND,
    parse::info::KIND,
];

/// The kinds that carry a `network` tag and so pass the filter of step 5.
const NETWORK_TAGGED: [u16; 2] = [parse::order::KIND, parse::dev_fee::KIND];

/// What became of one event.
///
/// Returned rather than logged-and-forgotten so that a caller walking a relay
/// backlog can report how much of it was new, how much it had already seen,
/// and how much it turned away — three numbers that say very different things
/// about a run.
#[derive(Debug, Clone, PartialEq)]
pub enum IngestOutcome {
    /// New to the archive, parsed and persisted.
    Stored,
    /// Already in the archive; nothing was written but this relay's cursor,
    /// which moves because the relay did deliver the event.
    Duplicate,
    /// Turned away, for the stated reason.
    Rejected(Rejection),
}

/// Why an event was not indexed.
///
/// Each variant carries what the decision was made on, because a rejection
/// count is only actionable if the log line says which rule fired and with
/// what value.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Rejection {
    #[error("the id or the signature does not match the event")]
    InvalidSignature,

    #[error("`{pubkey}` is not a configured instance")]
    UnknownInstance { pubkey: String },

    #[error(
        "published by {}, not by `{MOSTRO}`",
        .platform.as_deref().map_or_else(|| "no single platform".to_string(), |name| format!("`{name}`"))
    )]
    OtherPlatform { platform: Option<String> },

    #[error("published on `{network}`, which is not indexed")]
    OtherNetwork { network: Network },

    #[error("kind {kind} has no parser")]
    UnsupportedKind { kind: u16 },

    #[error("malformed: {0}")]
    Malformed(#[from] ParseError),
}

/// Which events this indexer wants, from `docs/SPEC.md` §9.
///
/// Held apart from [`Settings`](crate::config::Settings) so the pipeline
/// depends on the three answers it needs rather than on the whole
/// configuration file, and so a test can state a policy in one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Lowercase hex. Empty means the allow-list is not in use, which is only
    /// coherent together with `accept_unknown_instances`.
    instances: HashSet<String>,
    accept_unknown_instances: bool,
    networks: HashSet<Network>,
}

impl Policy {
    pub fn new(
        instances: impl IntoIterator<Item = String>,
        accept_unknown_instances: bool,
        networks: impl IntoIterator<Item = Network>,
    ) -> Self {
        Self {
            instances: instances
                .into_iter()
                .map(|pubkey| pubkey.trim().to_lowercase())
                .collect(),
            accept_unknown_instances,
            networks: networks.into_iter().collect(),
        }
    }

    /// Whether `pubkey` is one this indexer follows (step 3).
    fn accepts_instance(&self, pubkey: &str) -> bool {
        self.accept_unknown_instances || self.instances.contains(&pubkey.to_lowercase())
    }

    /// Whether `network` is one this indexer counts (step 5).
    fn accepts_network(&self, network: Network) -> bool {
        self.networks.contains(&network)
    }
}

impl From<&IndexerSettings> for Policy {
    fn from(settings: &IndexerSettings) -> Self {
        Self::new(
            settings.instances.iter().cloned(),
            settings.accept_unknown_instances,
            settings.networks.iter().copied(),
        )
    }
}

/// One published version of an event, parsed into whatever its kind means.
///
/// The variants exist so that parsing (which can fail on the event) happens
/// before the transaction opens, leaving the transaction with nothing to do
/// but write.
#[derive(Debug)]
enum Parsed {
    Order(Box<parse::order::OrderVersion>),
    DevFee(Box<parse::dev_fee::DevFee>),
    Dispute(Box<parse::dispute::DisputeVersion>),
    Info(Box<parse::info::InstanceInfo>),
}

/// The pipeline itself: a database and the policy to admit events by.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pool: SqlitePool,
    policy: Policy,
}

impl Pipeline {
    pub fn new(pool: SqlitePool, policy: Policy) -> Self {
        Self { pool, policy }
    }

    /// Runs `event`, delivered by `relay_url`, through the whole of §8.1.
    ///
    /// `now` is the wall clock the archive records as `seen_at`; it is a
    /// parameter so that ingesting the same backlog twice in a test produces
    /// the same rows.
    ///
    /// The `Err` case is reserved for the database being unusable. An event
    /// this indexer does not want is not an error — it is an answer, and it
    /// comes back as [`IngestOutcome::Rejected`].
    pub async fn ingest(
        &self,
        event: &Event,
        relay_url: &str,
        now: i64,
    ) -> Result<IngestOutcome, sqlx::Error> {
        if let Some(rejection) = self.admit(event) {
            tracing::debug!(id = %event.id, %rejection, "rejected");
            return Ok(IngestOutcome::Rejected(rejection));
        }

        // Step 7's parsing happens before anything is written: it reads the
        // event and nothing else, and doing it first is what lets the writes
        // that follow be a single transaction.
        let record = repo::events::EventRecord::new(event, relay_url, now);
        let parsed = match Self::parse(event) {
            Ok(parsed) => parsed,
            Err(rejection) => {
                // Step 6 alone. The event is kept so that a parser fixed later
                // can be run over the archive instead of over the relays.
                repo::events::insert_if_new(&self.pool, &record).await?;
                tracing::debug!(id = %event.id, %rejection, "archived but not parsed");
                return Ok(IngestOutcome::Rejected(rejection));
            }
        };

        let mut tx = self.pool.begin().await?;

        // Step 6.
        if !repo::events::insert_if_new(&mut *tx, &record).await? {
            // Step 8 still runs. Cursors are per relay, and `Subscription`
            // hands over each relay's copy of an event precisely so that the
            // relay that delivered this one is recorded as having reached it.
            // Skipping the cursor here would leave a second relay permanently
            // behind, re-requesting a backlog it has already sent.
            Self::advance(&mut tx, event, relay_url, now).await?;
            tx.commit().await?;
            return Ok(IngestOutcome::Duplicate);
        }

        // Step 7.
        Self::persist(&mut tx, event, &parsed).await?;

        // Step 8.
        Self::advance(&mut tx, event, relay_url, now).await?;

        tx.commit().await?;

        Ok(IngestOutcome::Stored)
    }

    /// Step 8: this relay has now been read as far as this event.
    async fn advance(
        tx: &mut Transaction<'_, Sqlite>,
        event: &Event,
        relay_url: &str,
        now: i64,
    ) -> Result<(), sqlx::Error> {
        repo::sync_state::advance(
            &mut **tx,
            relay_url,
            event.kind.as_u16(),
            event.created_at.as_secs() as i64,
            now,
        )
        .await
    }

    /// Steps 2 to 5: everything that can be decided from the event alone.
    ///
    /// `None` means the event is wanted. The order is the spec's: the
    /// signature first, because an unverified event's own pubkey is not
    /// evidence of anything.
    fn admit(&self, event: &Event) -> Option<Rejection> {
        if event.verify().is_err() {
            return Some(Rejection::InvalidSignature);
        }

        let pubkey = event.pubkey.to_hex();
        if !self.policy.accepts_instance(&pubkey) {
            return Some(Rejection::UnknownInstance { pubkey });
        }

        let kind = event.kind.as_u16();

        if PLATFORM_TAGGED.contains(&kind) && !parse::is_mostro(event) {
            return Some(Rejection::OtherPlatform {
                platform: parse::platform(event),
            });
        }

        if NETWORK_TAGGED.contains(&kind) {
            // A missing `network` tag is not a rejection. The tag is optional
            // on the wire and the column is nullable, so an event that does
            // not say where it lives is archived saying so, and the `--network`
            // filters downstream can leave it out. Turning it away here would
            // lose it from the archive for good: the relays only hold the
            // latest version of an addressable event.
            match parse::optional_network(event) {
                Ok(Some(network)) if !self.policy.accepts_network(network) => {
                    return Some(Rejection::OtherNetwork { network });
                }
                Ok(_) => {}
                Err(error) => return Some(Rejection::Malformed(error)),
            }
        }

        None
    }

    /// Step 7's first half: the event's kind decides which parser reads it.
    fn parse(event: &Event) -> Result<Parsed, Rejection> {
        let kind = event.kind.as_u16();

        match kind {
            parse::order::KIND => Ok(Parsed::Order(Box::new(parse::order::parse(event)?))),
            parse::dev_fee::KIND => Ok(Parsed::DevFee(Box::new(parse::dev_fee::parse(event)?))),
            parse::dispute::KIND => Ok(Parsed::Dispute(Box::new(parse::dispute::parse(event)?))),
            parse::info::KIND => Ok(Parsed::Info(Box::new(parse::info::parse(event)?))),
            kind => Err(Rejection::UnsupportedKind { kind }),
        }
    }

    /// Step 7's second half: the version and the projection it feeds, written
    /// into the caller's transaction.
    ///
    /// Together, because a version whose projection was never refreshed is a
    /// row that answers `orders` queries with the state before it — worse than
    /// not having stored it at all, and invisible until someone re-derives the
    /// numbers by hand.
    async fn persist(
        tx: &mut Transaction<'_, Sqlite>,
        event: &Event,
        parsed: &Parsed,
    ) -> Result<(), sqlx::Error> {
        let pubkey = event.pubkey.to_hex();
        let created_at = event.created_at.as_secs() as i64;
        let name = parse::instance_name(event);

        // Every indexed kind is published *by* an instance, so the bestiary
        // grows from all four rather than from 38385 alone: an instance that
        // never publishes its profile is still one this network has seen.
        repo::instances::upsert(&mut **tx, &pubkey, name.as_deref(), created_at).await?;
        if let Some(name) = &name {
            repo::instances::record_name(&mut **tx, &pubkey, name, created_at).await?;
        }

        match parsed {
            Parsed::Order(version) => {
                repo::orders::insert_version(&mut **tx, version).await?;
                repo::orders::refresh_projection(&mut **tx, &version.order_id).await?;
            }
            Parsed::DevFee(fee) => {
                repo::dev_fees::insert(&mut **tx, fee).await?;
            }
            Parsed::Dispute(version) => {
                repo::disputes::insert_version(&mut **tx, version).await?;
                repo::disputes::refresh_projection(&mut **tx, &version.dispute_id).await?;
            }
            Parsed::Info(info) => {
                repo::instance_info::insert_version(&mut **tx, info).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
