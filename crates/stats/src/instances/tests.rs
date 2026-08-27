use super::*;
use crate::activity::{Direction, Status};

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 2_500;

fn alpha() -> Profile {
    Profile {
        pubkey: "aaaa0000".into(),
        name: Some("Alpha".into()),
        label: "Alpha (aaaa0000)".into(),
        mostro_version: Some("0.14.0".into()),
        protocol_version: Some("1".into()),
        fee: Some(0.006),
        min_order_sats: Some(1_000),
        max_order_sats: Some(500_000),
        fiat_currencies: vec!["ARS".into(), "VES".into()],
        ln_networks: vec!["mainnet".into()],
        bond_enabled: Some(true),
        first_seen_at: 100,
        last_seen_at: 2_400,
    }
}

fn nameless() -> Profile {
    Profile {
        pubkey: "bbbb0000".into(),
        name: None,
        label: "bbbb0000".into(),
        mostro_version: None,
        protocol_version: None,
        fee: None,
        min_order_sats: None,
        max_order_sats: None,
        fiat_currencies: vec![],
        ln_networks: vec![],
        bond_enabled: None,
        first_seen_at: 100,
        last_seen_at: 200,
    }
}

fn order(id: &str, pubkey: &str, created_at: i64, status: Status, sats: i64) -> Order {
    Order {
        order_id: id.into(),
        pubkey: pubkey.into(),
        instance: pubkey.into(),
        created_at,
        status,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec![],
        amount_sats: sats,
        fiat_amount: None,
        taken_at: None,
        success_at: (status == Status::Success).then_some(created_at + 10),
        canceled_at: None,
        expires_at: None,
    }
}

fn value<'a>(metrics: &'a [Metric], name: &str) -> &'a Value {
    &metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("`{name}` is reported"))
        .value
}

#[test]
fn a_profile_renders_every_field_of_the_spec() {
    // Arrange / Act
    let metrics = profile_metrics("instance", &alpha(), NOW);

    // Assert
    assert_eq!(
        value(&metrics, "instance.name"),
        &Value::Text("Alpha".into())
    );
    assert_eq!(value(&metrics, "instance.fee"), &Value::Ratio(0.006));
    assert_eq!(value(&metrics, "instance.min_order"), &Value::Sats(1_000));
    assert_eq!(
        value(&metrics, "instance.fiat"),
        &Value::Text("ARS,VES".into())
    );
    assert_eq!(
        value(&metrics, "instance.bond"),
        &Value::Text("enabled".into())
    );
    assert_eq!(
        value(&metrics, "instance.first_seen"),
        &Value::Text("1970-01-01T00:01:40+00:00".into())
    );
    assert_eq!(value(&metrics, "instance.silent_for"), &Value::Seconds(100));
    assert_eq!(
        value(&metrics, "instance.silent"),
        &Value::Text("no".into())
    );
}

#[test]
fn what_an_instance_never_published_is_missing_not_blank() {
    let metrics = profile_metrics("instance", &nameless(), NOW);

    assert_eq!(value(&metrics, "instance.name"), &Value::Missing);
    assert_eq!(value(&metrics, "instance.fee"), &Value::Missing);
    assert_eq!(value(&metrics, "instance.fiat"), &Value::Missing);
    assert_eq!(value(&metrics, "instance.bond"), &Value::Missing);
}

#[test]
fn silence_is_more_than_a_week_without_any_event() {
    let mut profile = alpha();

    profile.last_seen_at = NOW - SILENT_AFTER_SECS;
    assert!(!profile.is_silent(NOW), "exactly a week is not yet silent");

    profile.last_seen_at = NOW - SILENT_AFTER_SECS - 1;
    assert!(profile.is_silent(NOW));
}

#[test]
fn the_list_has_one_block_per_instance_with_its_orders_created_in_the_window() {
    let orders = vec![
        order("a1", "aaaa0000", 1_100, Status::Pending, 0),
        order("a2", "aaaa0000", 1_200, Status::Pending, 0),
        order("a-before", "aaaa0000", 900, Status::Pending, 0),
        order("b1", "bbbb0000", 1_300, Status::Pending, 0),
    ];

    let metrics = list(&[alpha(), nameless()], &orders, WINDOW, NOW);

    assert_eq!(metrics.len(), 2 * 15);
    assert_eq!(metrics[0].name, "instances.Alpha (aaaa0000).pubkey");
    assert_eq!(
        value(&metrics, "instances.Alpha (aaaa0000).created"),
        &Value::Count(2)
    );
    assert_eq!(
        value(&metrics, "instances.bbbb0000.created"),
        &Value::Count(1)
    );
}

#[test]
fn the_profile_view_reports_the_instance_share_of_the_network() {
    let own = vec![
        order("a1", "aaaa0000", 1_100, Status::Success, 300),
        order("a2", "aaaa0000", 1_200, Status::Pending, 0),
    ];
    let mut network = own.clone();
    network.push(order("b1", "bbbb0000", 1_300, Status::Success, 700));
    network.push(order("b2", "bbbb0000", 1_400, Status::Pending, 0));

    let metrics = profile(
        &alpha(),
        &own,
        &network,
        &DevFeeData::default(),
        Some(&DisputeData::default()),
        WINDOW,
        NOW,
    );

    assert_eq!(
        value(&metrics, "instance.name"),
        &Value::Text("Alpha".into())
    );
    assert_eq!(value(&metrics, "orders.created"), &Value::Count(2));
    assert_eq!(value(&metrics, "volume.sats"), &Value::Sats(300));
    assert_eq!(value(&metrics, "dev_fees.paid"), &Value::Count(0));
    assert_eq!(value(&metrics, "disputes.opened"), &Value::Count(0));
    assert_eq!(value(&metrics, "share.orders"), &Value::Ratio(0.5));
    assert_eq!(value(&metrics, "share.volume"), &Value::Ratio(0.3));
}

#[test]
fn a_share_of_nothing_is_missing() {
    let metrics = profile(
        &alpha(),
        &[],
        &[],
        &DevFeeData::default(),
        Some(&DisputeData::default()),
        WINDOW,
        NOW,
    );

    assert_eq!(value(&metrics, "share.orders"), &Value::Missing);
    assert_eq!(value(&metrics, "share.volume"), &Value::Missing);
}

#[test]
fn every_bestiary_metric_is_observed() {
    let metrics = profile(
        &alpha(),
        &[],
        &[],
        &DevFeeData::default(),
        Some(&DisputeData::default()),
        WINDOW,
        NOW,
    );

    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn a_profile_whose_scope_cannot_reach_disputes_reports_them_as_missing() {
    let metrics = profile(
        &alpha(),
        &[],
        &[],
        &DevFeeData::default(),
        None,
        WINDOW,
        NOW,
    );

    assert_eq!(value(&metrics, "disputes.opened"), &Value::Missing);
    assert_eq!(value(&metrics, "disputes.open_now"), &Value::Missing);
    assert_eq!(value(&metrics, "orders.created"), &Value::Count(0));
}
