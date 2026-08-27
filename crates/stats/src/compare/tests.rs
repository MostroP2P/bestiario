use super::*;
use crate::activity::{Direction, Status};
use crate::dev_fees::Fee;
use crate::disputes::{Dispute, Taken};

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 2_500;

fn profile(label: &str, fee: Option<f64>, version: Option<&str>) -> Profile {
    Profile {
        pubkey: label.into(),
        name: None,
        label: label.into(),
        mostro_version: version.map(str::to_string),
        protocol_version: None,
        fee,
        min_order_sats: None,
        max_order_sats: None,
        fiat_currencies: vec![],
        ln_networks: vec![],
        bond_enabled: None,
        first_seen_at: 0,
        last_seen_at: NOW,
    }
}

fn order(id: &str, instance: &str, status: Status, sats: i64) -> Order {
    Order {
        order_id: id.into(),
        pubkey: instance.into(),
        instance: instance.into(),
        created_at: 1_100,
        status,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec![],
        amount_sats: sats,
        taken_at: Some(1_150),
        success_at: (status == Status::Success).then_some(1_200),
        canceled_at: (status == Status::Canceled).then_some(1_200),
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
fn one_row_per_instance_with_its_own_figures() {
    // Arrange: Alpha completed two of three (500 sats) and sent 40 sats of
    // dev fees; Beta completed one and had one dispute over two takers.
    let orders = vec![
        order("a1", "Alpha", Status::Success, 200),
        order("a2", "Alpha", Status::Success, 300),
        order("a3", "Alpha", Status::Canceled, 999),
        order("b1", "Beta", Status::Success, 700),
        order("b2", "Beta", Status::Pending, 999),
    ];
    let fees = DevFeeData {
        fees: vec![Fee {
            event_id: "f".into(),
            order_id: "a1".into(),
            instance: "Alpha".into(),
            created_at: 1_300,
            amount_sats: 40,
            is_duplicate: false,
            order_known: true,
            settled_at: Some(1_200),
        }],
        settlements: vec![],
    };
    let disputes = DisputeData {
        disputes: vec![Dispute {
            dispute_id: "d".into(),
            instance: "Beta".into(),
            opened_at: 1_400,
            status: disputes::Status::Initiated,
            initiator: None,
            resolved_at: None,
            outcome: None,
        }],
        taken: vec![
            Taken {
                order_id: "b1".into(),
                instance: "Beta".into(),
                left_pending_at: 1_150,
            },
            Taken {
                order_id: "b2".into(),
                instance: "Beta".into(),
                left_pending_at: 1_160,
            },
        ],
    };
    let profiles = [
        profile("Alpha", Some(0.006), Some("0.14.0")),
        profile("Beta", None, None),
    ];

    // Act
    let metrics = report(&profiles, &orders, &fees, Some(&disputes), WINDOW, NOW);

    // Assert
    assert_eq!(metrics.len(), 14);
    assert_eq!(value(&metrics, "compare.Alpha.completed"), &Value::Count(2));
    assert_eq!(
        value(&metrics, "compare.Alpha.volume_sats"),
        &Value::Sats(500)
    );
    assert_eq!(
        value(&metrics, "compare.Alpha.completion_rate"),
        &Value::Ratio(2.0 / 3.0)
    );
    assert_eq!(value(&metrics, "compare.Alpha.fee"), &Value::Ratio(0.006));
    assert_eq!(
        value(&metrics, "compare.Alpha.dev_fees_sats"),
        &Value::Sats(40)
    );
    assert_eq!(
        value(&metrics, "compare.Alpha.dispute_rate"),
        &Value::Missing
    );
    assert_eq!(
        value(&metrics, "compare.Alpha.version"),
        &Value::Text("0.14.0".into())
    );

    assert_eq!(value(&metrics, "compare.Beta.completed"), &Value::Count(1));
    assert_eq!(
        value(&metrics, "compare.Beta.dev_fees_sats"),
        &Value::Sats(0)
    );
    assert_eq!(
        value(&metrics, "compare.Beta.dispute_rate"),
        &Value::Ratio(0.5)
    );
    assert_eq!(value(&metrics, "compare.Beta.fee"), &Value::Missing);
    assert_eq!(value(&metrics, "compare.Beta.version"), &Value::Missing);
}

#[test]
fn an_instance_with_nothing_in_the_window_still_has_a_row() {
    let metrics = report(
        &[profile("Quiet", None, None)],
        &[],
        &DevFeeData::default(),
        Some(&DisputeData::default()),
        WINDOW,
        NOW,
    );

    assert_eq!(metrics.len(), 7);
    assert_eq!(value(&metrics, "compare.Quiet.completed"), &Value::Count(0));
    assert_eq!(
        value(&metrics, "compare.Quiet.completion_rate"),
        &Value::Missing
    );
}

#[test]
fn the_columns_are_the_figures_of_a_row_in_order() {
    let metrics = report(
        &[profile("Only", None, None)],
        &[],
        &DevFeeData::default(),
        Some(&DisputeData::default()),
        WINDOW,
        NOW,
    );

    let suffixes: Vec<&str> = metrics
        .iter()
        .map(|metric| metric.name.rsplit('.').next().expect("segment"))
        .collect();
    assert_eq!(suffixes, COLUMNS);
}

#[test]
fn a_scope_that_cannot_reach_disputes_leaves_every_dispute_rate_missing() {
    let metrics = report(
        &[profile("Alpha", None, None)],
        &[order("a1", "Alpha", Status::Success, 1)],
        &DevFeeData::default(),
        None,
        WINDOW,
        NOW,
    );

    assert_eq!(
        value(&metrics, "compare.Alpha.dispute_rate"),
        &Value::Missing
    );
    assert_eq!(value(&metrics, "compare.Alpha.completed"), &Value::Count(1));
}
