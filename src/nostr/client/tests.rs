//! Tests against a real relay, not a mock of one.
//!
//! `nostr-sdk`'s `local-relay` feature runs an actual relay over a websocket
//! on localhost, so these exercise the connection, subscription and REQ paths
//! rather than a hand-written stand-in for them. What is being tested here is
//! mostly *how this crate handles what a relay does* — an empty window, a
//! relay that is not there, the same event arriving twice — and a mock built
//! from the same assumptions the code makes would agree with the code by
//! construction.

use std::time::Duration;

use nostr_sdk::local_relay::LocalRelay;

use super::*;

/// Long enough that a busy CI machine does not fail the test, short enough
/// that a genuine hang is not mistaken for slowness.
const PATIENCE: Duration = Duration::from_secs(5);

/// A port nothing is listening on. Port 1 is reserved and unroutable in
/// practice, so a connection attempt fails rather than hanging.
const DEAD_RELAY: &str = "ws://127.0.0.1:1";

async fn relay() -> MockRelay {
    MockRelay::run().await.expect("start the local relay")
}

/// A signed kind 38383 event at `created_at`, from a throwaway instance key.
fn order_at(keys: &Keys, created_at: u64) -> Event {
    EventBuilder::new(Kind::from_u16(38383), "")
        .tag(Tag::identifier(format!("order-{created_at}")))
        .custom_created_at(Timestamp::from_secs(created_at))
        .finalize(keys)
        .expect("sign")
}

/// Publishes `events` into `relay` and waits for the relay to have stored
/// them, so a later fetch is not racing the write.
async fn seed(relay: &MockRelay, events: &[Event]) {
    for event in events {
        relay
            .add_event(event.clone())
            .await
            .expect("the local relay accepts a signed event");
    }
}

#[tokio::test]
async fn a_window_returns_the_events_that_fall_inside_it() {
    // Arrange: three orders one hour apart, and a window over the middle one.
    let relay = relay().await;
    let keys = Keys::generate();
    let events = [
        order_at(&keys, 1_000),
        order_at(&keys, 4_600),
        order_at(&keys, 8_200),
    ];
    seed(&relay, &events).await;

    let client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");

    // Act
    let filter = Filter::new()
        .kind(Kind::from_u16(38383))
        .since(Timestamp::from_secs(4_000))
        .until(Timestamp::from_secs(5_000));
    let fetched = client
        .fetch_window(&client.relays()[0], filter)
        .await
        .expect("fetch");

    // Assert
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].id, events[1].id);
}

#[tokio::test]
async fn a_window_with_nothing_in_it_comes_back_empty_rather_than_erroring() {
    // This is the stop condition of the backwards walk in `docs/SPEC.md` §8.2:
    // backfill keeps stepping into the past until a window comes back empty.
    // If "no events" were an error the walk could not tell it apart from a
    // relay that had failed, and would stop early on a live relay.
    let relay = relay().await;
    let client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");

    let fetched = client
        .fetch_window(
            &client.relays()[0],
            Filter::new().kind(Kind::from_u16(38383)),
        )
        .await
        .expect("an empty window is not a failure");

    assert!(fetched.is_empty());
}

#[tokio::test]
async fn a_window_is_returned_newest_first() {
    // The backwards walk builds its next `until` from the oldest event it
    // received, so the ordering of this list is load-bearing.
    let relay = relay().await;
    let keys = Keys::generate();
    seed(
        &relay,
        &[
            order_at(&keys, 1_000),
            order_at(&keys, 3_000),
            order_at(&keys, 2_000),
        ],
    )
    .await;

    let client = RelayClient::connect(&[relay.url().await.to_string()])
        .await
        .expect("connect");
    let fetched = client
        .fetch_window(
            &client.relays()[0],
            Filter::new().kind(Kind::from_u16(38383)),
        )
        .await
        .expect("fetch");

    let timestamps: Vec<u64> = fetched.iter().map(|e| e.created_at.as_secs()).collect();
    assert_eq!(timestamps, vec![3_000, 2_000, 1_000]);
}

#[tokio::test]
async fn a_relay_that_is_not_there_does_not_stop_the_ones_that_are() {
    // A run against five relays where one is down has to index the other
    // four, not abort. This is the whole reason `connect` reports a partial
    // set instead of a `Result` per relay.
    let live = relay().await;
    let live_url = live.url().await;

    let client = RelayClient::connect(&[DEAD_RELAY.to_string(), live_url.to_string()])
        .await
        .expect("one reachable relay is enough");

    assert_eq!(client.relays(), &[live_url]);
}

#[tokio::test]
async fn connecting_when_no_relay_answers_is_an_error() {
    // The opposite case: continuing with an empty relay set would run a
    // backfill that reads nothing and reports success.
    let error = RelayClient::connect(&[DEAD_RELAY.to_string()])
        .await
        .expect_err("no relay is reachable");

    assert!(matches!(
        error,
        ClientError::NoRelayReachable { attempted: 1 }
    ));
}

#[tokio::test]
async fn a_malformed_relay_url_is_skipped_like_an_unreachable_one() {
    // Configured relays are validated at startup, but relays discovered over
    // NIP-65 (roadmap PR 40) are not: they arrive from a third party and one
    // bad entry must not take the run down.
    let live = relay().await;
    let live_url = live.url().await;

    let client = RelayClient::connect(&["not a url at all".to_string(), live_url.to_string()])
        .await
        .expect("the usable relay is still usable");

    assert_eq!(client.relays(), &[live_url]);
}

#[tokio::test]
async fn a_relay_is_asked_only_for_its_own_window() {
    // Cursors are per `(relay, kind)`, so a fetch aimed at one relay must not
    // quietly answer with another relay's events.
    let first = relay().await;
    let second = relay().await;
    let keys = Keys::generate();
    let only_on_first = order_at(&keys, 1_000);
    let only_on_second = order_at(&keys, 2_000);
    seed(&first, std::slice::from_ref(&only_on_first)).await;
    seed(&second, std::slice::from_ref(&only_on_second)).await;

    let client = RelayClient::connect(&[
        first.url().await.to_string(),
        second.url().await.to_string(),
    ])
    .await
    .expect("connect");

    let from_first = client
        .fetch_window(
            &first.url().await,
            Filter::new().kind(Kind::from_u16(38383)),
        )
        .await
        .expect("fetch");

    assert_eq!(
        from_first.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![only_on_first.id]
    );
}

#[tokio::test]
async fn a_live_subscription_delivers_an_event_published_after_it_started() {
    let relay = relay().await;
    let url = relay.url().await;
    let client = RelayClient::connect(&[url.to_string()])
        .await
        .expect("connect");

    let mut subscription = client
        .subscribe(vec![(
            url.clone(),
            vec![Filter::new().kind(Kind::from_u16(38383))],
        )])
        .await
        .expect("subscribe");

    let keys = Keys::generate();
    let published = order_at(&keys, 1_000);
    seed(&relay, std::slice::from_ref(&published)).await;

    let (from, event) = tokio::time::timeout(PATIENCE, subscription.next_event())
        .await
        .expect("an event arrives")
        .expect("the stream is still open");

    assert_eq!(from, url);
    assert_eq!(event.id, published.id);
}

#[tokio::test]
async fn the_same_event_on_two_relays_is_reported_once_per_relay() {
    // Each relay carries its own cursor, so an event seen on both has to
    // advance both. A stream that deduplicated by event id — which is what
    // the sdk's own `Event` notification does — would advance whichever relay
    // happened to answer first and leave the other one permanently behind.
    let first = relay().await;
    let second = relay().await;
    let (first_url, second_url) = (first.url().await, second.url().await);

    let client = RelayClient::connect(&[first_url.to_string(), second_url.to_string()])
        .await
        .expect("connect");

    let filter = Filter::new().kind(Kind::from_u16(38383));
    let mut subscription = client
        .subscribe(vec![
            (first_url.clone(), vec![filter.clone()]),
            (second_url.clone(), vec![filter]),
        ])
        .await
        .expect("subscribe");

    let keys = Keys::generate();
    let shared = order_at(&keys, 1_000);
    seed(&first, std::slice::from_ref(&shared)).await;
    seed(&second, std::slice::from_ref(&shared)).await;

    let mut seen_on = Vec::new();
    for _ in 0..2 {
        let (from, event) = tokio::time::timeout(PATIENCE, subscription.next_event())
            .await
            .expect("both relays report it")
            .expect("the stream is still open");
        assert_eq!(event.id, shared.id);
        seen_on.push(from);
    }

    seen_on.sort();
    let mut expected = vec![first_url, second_url];
    expected.sort();
    assert_eq!(seen_on, expected);
}

#[tokio::test]
async fn a_relay_that_was_down_at_startup_is_picked_up_when_it_comes_back() {
    // The case this exists for: one configured relay is down when the indexer
    // starts, another is up, and the run continues on the one that answered.
    // Without a retry the first is ignored for the lifetime of the process,
    // and whatever only it carries is never indexed.
    let port = free_port();
    let down = format!("ws://127.0.0.1:{port}");
    let live = relay().await;
    let live_url = live.url().await;

    let mut client = RelayClient::connect(&[down.clone(), live_url.to_string()])
        .await
        .expect("one reachable relay is enough");
    assert_eq!(client.relays(), std::slice::from_ref(&live_url));

    // Act: the relay comes back on the address it was configured under.
    let recovered = LocalRelay::builder().port(port).build();
    recovered.run().await.expect("start the recovered relay");
    client.reattach().await;

    // Assert: it is in play, and in the order it was configured in.
    assert_eq!(client.relays(), &[recovered.url().await, live_url]);
}

#[tokio::test]
async fn reattaching_when_nothing_has_changed_leaves_the_relay_list_alone() {
    // Called before every resubscription, so it has to be free of side
    // effects when there is nothing to recover: no duplicates, no reordering,
    // no dropping a relay that is working.
    let live = relay().await;
    let live_url = live.url().await;
    let mut client = RelayClient::connect(&[DEAD_RELAY.to_string(), live_url.to_string()])
        .await
        .expect("connect");

    client.reattach().await;
    client.reattach().await;

    assert_eq!(client.relays(), &[live_url]);
}

/// A port with nothing on it *yet*: bound to find a free one, then released
/// so the relay under test can take it. Racy in principle, reliable in a test
/// process that binds it back a moment later.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}
