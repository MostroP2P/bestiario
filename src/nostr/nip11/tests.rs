use super::*;

#[test]
fn a_websocket_url_becomes_the_http_address_nip11_is_served_from() {
    assert_eq!(http_url("wss://relay.example"), "https://relay.example");
    assert_eq!(
        http_url("wss://relay.example/nostr"),
        "https://relay.example/nostr",
        "the path is part of the address, not decoration"
    );
    assert_eq!(
        http_url("ws://127.0.0.1:7000"),
        "http://127.0.0.1:7000",
        "a local relay is not served over TLS, and the E2E suite runs against one"
    );
}

#[test]
fn a_url_with_no_scheme_is_left_alone() {
    // Validation refuses it at startup; rewriting it here would turn a
    // configuration error into a request to somewhere unintended.
    assert_eq!(http_url("relay.example"), "relay.example");
}

#[tokio::test]
async fn a_relay_that_cannot_be_asked_advertises_nothing() {
    // Port 1 on the loopback: nothing listens, and the answer has to be
    // "no limit stated" rather than a failure, or one unreachable relay
    // would make a snapshot unreviewable.
    let advertised = limits(&["ws://127.0.0.1:1".to_string()]).await;

    assert_eq!(
        advertised,
        vec![Advertised {
            relay: "ws://127.0.0.1:1".to_string(),
            max_content_length: None,
        }]
    );
}

#[tokio::test]
async fn every_relay_is_asked_in_the_order_it_was_configured() {
    let relays = vec![
        "ws://127.0.0.1:1".to_string(),
        "ws://127.0.0.1:2".to_string(),
    ];

    let advertised = limits(&relays).await;

    assert_eq!(
        advertised
            .iter()
            .map(|a| a.relay.clone())
            .collect::<Vec<_>>(),
        relays,
        "a --dry-run listing is read against the configuration file"
    );
}
