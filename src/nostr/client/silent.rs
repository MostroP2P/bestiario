//! A relay that takes every event and never says what became of it.
//!
//! The failure a `publish` run actually met in production: the websocket
//! opens, the `EVENT` is written, and the `OK` never comes. A relay that is
//! down refuses at once and costs nothing; this one costs the whole wait for
//! the `OK`, once per event, which is what makes it worth a fixture of its
//! own. `MockRelay`'s own unresponsive option stalls the handshake instead,
//! so `connect` drops it before anything is ever sent to it.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use nostr_sdk::local_relay::{LocalRelay, WritePolicy, WritePolicyResult};
use nostr_sdk::prelude::Event;

#[derive(Debug)]
struct NeverAnswers;

impl WritePolicy for NeverAnswers {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _addr: &'a SocketAddr,
    ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}

/// A running relay that accepts connections and never answers an `EVENT`.
pub(crate) async fn relay() -> LocalRelay {
    let relay = LocalRelay::builder().write_policy(NeverAnswers).build();
    relay.run().await.expect("start the silent relay");
    relay
}
