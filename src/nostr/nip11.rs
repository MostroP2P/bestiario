//! What a relay says about itself — NIP-11, and the one field publication
//! depends on (`docs/NOSTR-PUBLICATION.md` §9.1).
//!
//! A relay advertises `limitation.max_content_length` in a JSON document
//! served over HTTP from the same host as the websocket. It is read once,
//! before anything is signed, so that a document too large for a relay is
//! an error naming the document rather than a silent rejection — which
//! would leave an index naming a document that is not there.
//!
//! # A relay that does not answer
//!
//! NIP-11 is optional, and plenty of relays serve nothing at all. A relay
//! that cannot be asked, or that advertises no limit, therefore reports
//! `None` and does not lower the ceiling: the configured
//! `[publish].max_content_bytes` still applies to every document. The
//! alternative — refusing to *review* a snapshot because one relay is
//! down — would make `--dry-run` unavailable exactly when an operator
//! needs it. What each relay said is printed, so nothing about this is
//! silent.

use std::time::Duration;

use nostr_sdk::prelude::RelayInformationDocument;

/// How long a relay has to answer before it counts as unreachable. Short
/// on purpose: this is a metadata read on the way to the real work.
const TIMEOUT: Duration = Duration::from_secs(5);

/// What one relay advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertised {
    pub relay: String,
    /// `limitation.max_content_length`, when the relay states one.
    pub max_content_length: Option<usize>,
}

/// Every configured relay, asked in turn.
///
/// In the order given, because the operator wrote that order and a
/// `--dry-run` listing is read against the configuration file.
pub async fn limits(relays: &[String]) -> Vec<Advertised> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().ok();

    let mut advertised = Vec::with_capacity(relays.len());
    for relay in relays {
        advertised.push(Advertised {
            relay: relay.clone(),
            max_content_length: match &client {
                Some(client) => limit_of(client, relay).await,
                None => None,
            },
        });
    }
    advertised
}

/// `limitation.max_content_length` of one relay, or `None` for anything
/// that is not a number this publisher can use: no answer, no document,
/// no limit stated, or a negative one.
async fn limit_of(client: &reqwest::Client, relay: &str) -> Option<usize> {
    let json = client
        .get(http_url(relay))
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    RelayInformationDocument::from_json(&json)
        .ok()?
        .limitation?
        .max_content_length
        .and_then(|limit| usize::try_from(limit).ok())
}

/// The websocket URL as the address NIP-11 is served from: same host,
/// same path, HTTP scheme.
fn http_url(relay: &str) -> String {
    match relay.split_once("://") {
        Some(("wss", rest)) => format!("https://{rest}"),
        Some(("ws", rest)) => format!("http://{rest}"),
        // Validation refuses anything else at startup; left alone rather
        // than guessed at, so a URL this does not understand fails as a
        // request rather than as a silently rewritten one.
        _ => relay.to_string(),
    }
}

#[cfg(test)]
mod tests;
