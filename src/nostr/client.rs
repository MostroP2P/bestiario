//! Relay connections, live subscriptions and the paginated backfill walk.
//!
//! Responsibility: get signed events off the wire and say which relay each one
//! came from. Nothing here verifies, parses or stores anything — that is
//! `ingest`.
//!
//! # Why every result is labelled with a relay
//!
//! The resume cursor of `repo::sync_state` is keyed by `(relay, kind)`
//! (`docs/SPEC.md` §8.2), so an event that arrives without knowing which relay
//! sent it cannot advance anything. That single requirement shapes this whole
//! module: [`RelayClient::fetch_window`] asks one relay rather than the pool,
//! and [`Subscription`] reports `(relay, event)` pairs rather than events.
//!
//! # Why a relay being down is not an error
//!
//! bestiario reads the same events from several relays on purpose; that is
//! what makes the index survive any one of them expiring its history. A run
//! that aborted because the third of five relays refused a connection would
//! throw away the four that answered. So [`RelayClient::connect`] keeps what
//! it could reach, logs what it could not, and fails only when *nothing* is
//! left — because indexing zero relays and reporting success is the one
//! outcome an operator cannot detect.

use std::str::FromStr;
use std::time::Duration;

use nostr_sdk::prelude::*;

#[cfg(test)]
mod tests;

/// How long to wait for a relay's websocket handshake before giving up on it.
///
/// Short on purpose: this is paid once per unreachable relay at start-up,
/// while the operator is watching.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a single backfill window may take before it is abandoned.
///
/// A window is one REQ that ends at EOSE, so this bounds a relay that accepts
/// the subscription and then never answers. The backfill walk retries by
/// asking for the same window again on the next run; the cursor has not moved.
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Anything that can go wrong between a list of relay URLs and a stream of
/// events.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(
        "none of the {attempted} configured relays could be reached; \
         nothing would be indexed"
    )]
    NoRelayReachable { attempted: usize },

    #[error("relay `{relay}` did not answer the request")]
    Fetch {
        relay: RelayUrl,
        // Boxed: `nostr_sdk::error::Error` is large enough that carrying it
        // inline makes every `Result` in this module the size of its error.
        #[source]
        source: Box<nostr_sdk::error::Error>,
    },

    #[error("could not subscribe to any relay")]
    Subscribe {
        #[source]
        source: Box<nostr_sdk::error::Error>,
    },

    /// Not "a relay refused the event", which is an outcome
    /// [`Delivery`] carries, but "the event could not be sent at all".
    #[error("event `{event}` could not be sent to any relay")]
    Send {
        event: EventId,
        #[source]
        source: Box<nostr_sdk::error::Error>,
    },
}

/// What became of one event, relay by relay.
///
/// Both halves are kept because both are worth printing: an operator who
/// publishes to five relays wants to know that the fifth has been refusing
/// everything for a week, and that is invisible if only the failures of a
/// document nobody took are reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub accepted: Vec<RelayUrl>,
    /// Each with the relay's own `OK` message, which is the only
    /// explanation there is.
    pub refused: Vec<(RelayUrl, String)>,
}

impl Delivery {
    /// Whether any relay took it. A document no relay took is not
    /// published, whatever the reasons the others gave.
    pub fn is_published(&self) -> bool {
        !self.accepted.is_empty()
    }
}

/// A pool of connected relays, each addressable on its own.
#[derive(Debug)]
pub struct RelayClient {
    client: Client,
    /// Every relay the operator asked for, kept verbatim so that one which was
    /// down at startup can be tried again. See [`RelayClient::reattach`].
    configured: Vec<String>,
    /// The relays that actually answered, in the order they were configured.
    /// Callers iterate this rather than the configured list, so a relay that
    /// is down is simply not walked.
    relays: Vec<RelayUrl>,
}

impl RelayClient {
    /// Connects to as many of `relays` as will answer.
    ///
    /// A URL that does not parse, or a relay that refuses the connection, is
    /// logged and dropped. The error is reserved for the case where that
    /// leaves nothing at all.
    pub async fn connect(relays: &[String]) -> Result<Self, ClientError> {
        let client = Client::default();
        let mut connected = Vec::new();

        for url in relays {
            match Self::add(&client, url).await {
                // Listing a relay twice — or under two spellings that
                // normalise to one URL — is one relay, not two. Keeping both
                // would subscribe to it twice and walk its history twice.
                Ok(url) if connected.contains(&url) => {
                    tracing::debug!(relay = %url, "configured more than once");
                }
                Ok(url) => connected.push(url),
                Err(reason) => tracing::warn!(relay = %url, %reason, "skipping relay"),
            }
        }

        if connected.is_empty() {
            return Err(ClientError::NoRelayReachable {
                attempted: relays.len(),
            });
        }

        tracing::info!(
            connected = connected.len(),
            configured = relays.len(),
            "connected to relays"
        );

        Ok(Self {
            client,
            configured: relays.to_vec(),
            relays: connected,
        })
    }

    /// Tries the configured relays that are not in play, and keeps the ones
    /// that answer this time.
    ///
    /// A relay that was down when the indexer started is not down for good,
    /// and nothing else would ever pick it up again: [`connect`](Self::connect)
    /// drops it, and every subscription and every backfill walk is built from
    /// the relays that answered. A long-running `sync` would ignore it until
    /// somebody restarted the process — and miss whatever only it carries.
    ///
    /// Rebuilds the list rather than appending to it, so the configured order
    /// [`relays`](Self::relays) promises survives a relay rejoining late, and
    /// a relay configured twice still appears once.
    pub async fn reattach(&mut self) {
        let configured = self.configured.clone();
        let mut attached = Vec::with_capacity(configured.len());

        for url in &configured {
            // A URL that does not parse was reported at startup and will not
            // start parsing; re-reporting it once a minute says nothing new.
            let Ok(parsed) = RelayUrl::from_str(url) else {
                continue;
            };

            if attached.contains(&parsed) {
                continue;
            }

            if self.relays.contains(&parsed) {
                attached.push(parsed);
                continue;
            }

            match Self::add(&self.client, url).await {
                Ok(url) => {
                    tracing::info!(relay = %url, "relay is answering again");
                    attached.push(url);
                }
                Err(reason) => tracing::debug!(relay = %url, %reason, "relay still unreachable"),
            }
        }

        self.relays = attached;
    }

    /// Replaces the set this client considers its own, for the next
    /// [`reattach`](Self::reattach) to dial.
    ///
    /// The connection set is not a fact of startup. A kind 10002 ingested
    /// while a run is under way names a relay an instance publishes to, and
    /// a client built once from the table as it stood would ignore it until
    /// the process was restarted — missing whatever only that relay
    /// carries. Callers hand back the recomputed set; nothing is dialled
    /// here, so a caller can compare before paying for a connection.
    ///
    /// A relay already attached and still named stays attached: this only
    /// changes what `reattach` will try.
    pub fn reconfigure(&mut self, relays: &[String]) {
        self.configured = relays.to_vec();
    }

    /// The relays named but not attached, normalised as they would be
    /// dialled.
    ///
    /// What a caller asks before deciding a reconnection is worth it: an
    /// empty answer means the client is already dialling everything it has
    /// been told about.
    pub fn unattached(&self) -> Vec<RelayUrl> {
        let mut missing = Vec::new();
        for url in &self.configured {
            let Ok(parsed) = RelayUrl::from_str(url) else {
                continue;
            };
            if !self.relays.contains(&parsed) && !missing.contains(&parsed) {
                missing.push(parsed);
            }
        }
        missing
    }

    /// Parses and connects one relay, returning why it was skipped if it was.
    ///
    /// The reason is a `String` because the two failures come from unrelated
    /// error types and the only thing done with either is to log it.
    async fn add(client: &Client, url: &str) -> Result<RelayUrl, String> {
        let url = RelayUrl::from_str(url).map_err(|e| e.to_string())?;

        client
            .add_relay(&url)
            .await
            .map_err(|e: Error| e.to_string())?;
        client
            .try_connect_relay(&url, CONNECT_TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;

        Ok(url)
    }

    /// The relays that answered, in configured order.
    pub fn relays(&self) -> &[RelayUrl] {
        &self.relays
    }

    /// One window of history from one relay: every event matching `filter`,
    /// newest first.
    ///
    /// Ends at the relay's EOSE, so an exhausted window returns an empty
    /// vector rather than an error — that is the stop condition of the
    /// backwards walk in `docs/SPEC.md` §8.2, and the walk has to be able to
    /// tell it apart from a relay that failed.
    ///
    /// Newest first because the caller builds the next window's `until` from
    /// the oldest event of this one.
    pub async fn fetch_window(
        &self,
        relay: &RelayUrl,
        filter: Filter,
    ) -> Result<Vec<Event>, ClientError> {
        let events = self
            .client
            .fetch_events(vec![(relay.clone(), vec![filter])])
            .timeout(FETCH_TIMEOUT)
            .await
            .map_err(|source| ClientError::Fetch {
                relay: relay.clone(),
                source: Box::new(source),
            })?;

        // `Events` is already ordered newest first; collecting into a `Vec`
        // fixes that ordering into the type rather than leaving it as a
        // property of the collection the caller happens to receive.
        Ok(events.into_iter().collect())
    }

    /// Opens a live subscription, each relay with its own filters.
    ///
    /// Per-relay filters rather than one shared set because `since` comes from
    /// that relay's cursor: relays are at different depths, and a shared
    /// `since` would either re-read what one of them had already given or skip
    /// what another had not.
    pub async fn subscribe(
        &self,
        targets: Vec<(RelayUrl, Vec<Filter>)>,
    ) -> Result<Subscription, ClientError> {
        // Taken *before* the REQ goes out. The channel only carries what
        // arrives after it is opened, so subscribing first would drop
        // everything the relay sends between the REQ and this line.
        let notifications = self.client.notifications();

        let output =
            self.client
                .subscribe(targets)
                .await
                .map_err(|source| ClientError::Subscribe {
                    source: Box::new(source),
                })?;

        Ok(Subscription {
            id: output.id().clone(),
            notifications,
        })
    }

    /// Closes one subscription, leaving the connections open.
    ///
    /// A `sync` that rebuilds its subscription — because the relays it
    /// should be following have changed — is not disconnecting, so the REQ
    /// it is replacing has to be closed by name. Left open, the relay would
    /// go on sending events for a subscription id nobody reads.
    /// A relay that will not take the CLOSE is logged and left: the
    /// subscription is being abandoned either way, and a failure to tidy up
    /// after it is not a reason to stop following the others.
    pub async fn close(&self, subscription: Subscription) {
        if let Err(error) = self.client.unsubscribe(&subscription.id).await {
            tracing::debug!(%error, "could not close the subscription");
        }
    }

    /// Sends one signed event to every connected relay, and says which of
    /// them took it.
    ///
    /// A relay that refuses is reported rather than raised: publication is
    /// to several relays on purpose, and a snapshot that reached four of
    /// five is a snapshot that is readable. The caller decides what a
    /// document nobody took means — for `publish`, it means the index that
    /// would name it is not sent (§7).
    ///
    /// The `Err` is reserved for not being able to send at all, which is
    /// not a per-relay outcome and would otherwise be reported as every
    /// relay having refused for the same reason.
    pub async fn send(&self, event: &Event) -> Result<Delivery, ClientError> {
        self.send_to(event, &self.relays).await
    }

    /// The same, to a chosen subset of the pool.
    ///
    /// The index of §7 is the caller for this: it may only go to relays
    /// that took every document it names, and a relay that took some of
    /// them is not one of those.
    pub async fn send_to(
        &self,
        event: &Event,
        relays: &[RelayUrl],
    ) -> Result<Delivery, ClientError> {
        let output = self
            .client
            .send_event(event)
            .to(relays.to_vec())
            .await
            .map_err(|source| ClientError::Send {
                event: event.id,
                source: Box::new(source),
            })?;

        // Sorted, so a listing of what happened reads in a stable order
        // rather than in the order a HashMap chose.
        let mut accepted: Vec<RelayUrl> = output.success.into_keys().collect();
        accepted.sort();
        let mut refused: Vec<(RelayUrl, String)> = output.failed.into_iter().collect();
        refused.sort();

        Ok(Delivery { accepted, refused })
    }

    /// Closes every connection, so a `sync` interrupted with SIGINT leaves no
    /// half-open websocket behind.
    pub async fn shutdown(self) {
        self.client.shutdown().await;
    }
}

/// What a subscription hands over, and which relay it came from.
///
/// Both carry the relay because both are about *that* relay's progress: an
/// event it holds, or the end of the history it holds. The pool's view of
/// either would be the wrong unit — cursors are per relay.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// An event the relay is delivering under this subscription.
    Event(RelayUrl, Event),
    /// The relay has sent everything it had stored for this subscription;
    /// what follows from it is live.
    EndOfStored(RelayUrl),
}

/// A live subscription, read one `(relay, event)` pair at a time.
pub struct Subscription {
    id: SubscriptionId,
    notifications: std::pin::Pin<Box<dyn futures::Stream<Item = ClientNotification> + Send>>,
}

impl Subscription {
    /// The next event, and the relay it came from, or `None` once the client
    /// has shut down.
    ///
    /// Reads the raw `Message` notification rather than the `Event` one on
    /// purpose. The sdk emits `Event` only the first time an event id is
    /// seen — across the whole pool — so an event carried by three relays
    /// would advance one cursor and leave the other two behind forever. Every
    /// relay that sends it is a relay that has reached that point in its own
    /// history, and dedup is the pipeline's job (`docs/SPEC.md` §8.1 step 6),
    /// not the transport's.
    pub async fn next_event(&mut self) -> Option<(RelayUrl, Event)> {
        loop {
            match self.next_incoming().await? {
                Incoming::Event(relay, event) => return Some((relay, event)),
                Incoming::EndOfStored(_) => continue,
            }
        }
    }

    /// The next thing this subscription has to say: an event, or a relay
    /// reporting that it has sent everything it had stored.
    ///
    /// EOSE is what tells a caller that a relay's *replay* is over and
    /// everything after it is live. A caller that has to read one set of
    /// kinds before asking for another — see `commands::sync` — has no other
    /// way to know when the first set is done.
    pub async fn next_incoming(&mut self) -> Option<Incoming> {
        use futures::StreamExt as _;

        while let Some(notification) = self.notifications.next().await {
            let ClientNotification::Message { relay_url, message } = notification else {
                continue;
            };

            match *message {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    // The client is shared, so another REQ's events arrive
                    // here too.
                    if subscription_id.as_ref() != &self.id {
                        continue;
                    }
                    return Some(Incoming::Event(relay_url, event.into_owned()));
                }
                RelayMessage::EndOfStoredEvents(subscription_id) => {
                    if subscription_id.as_ref() != &self.id {
                        continue;
                    }
                    return Some(Incoming::EndOfStored(relay_url));
                }
                _ => continue,
            }
        }

        None
    }
}
