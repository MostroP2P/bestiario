use super::*;
use crate::activity::{Direction, Status};
use crate::disputes::{Dispute, Initiator};

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 2_500;

fn order(id: &str, pubkey: &str, fiat: &str, methods: &[&str], status: Status, sats: i64) -> Order {
    Order {
        order_id: id.into(),
        pubkey: pubkey.into(),
        instance: pubkey.into(),
        created_at: 1_100,
        status,
        direction: Direction::Buy,
        fiat_code: fiat.into(),
        payment_methods: methods.iter().map(|m| m.to_string()).collect(),
        amount_sats: sats,
        taken_at: None,
        success_at: (status == Status::Success).then_some(1_200),
        canceled_at: (status == Status::Canceled).then_some(1_200),
        expires_at: None,
    }
}

/// Five orders from two instances: three completed (600 sats), one
/// canceled, one pending; ARS three times, VES twice; cash four times,
/// bank twice, zelle once. One open dispute.
fn orders() -> Vec<Order> {
    vec![
        order("o1", "a", "ARS", &["cash"], Status::Success, 100),
        order("o2", "a", "ARS", &["cash", "bank"], Status::Success, 200),
        order("o3", "b", "VES", &["cash", "zelle"], Status::Success, 300),
        order("o4", "b", "VES", &["bank"], Status::Canceled, 999),
        order("o5", "a", "ARS", &["cash"], Status::Pending, 999),
        // Before the window: in nothing.
        Order {
            created_at: 500,
            success_at: Some(600),
            ..order("old", "c", "USD", &["wire"], Status::Success, 999)
        },
    ]
}

fn disputes() -> DisputeData {
    DisputeData {
        disputes: vec![Dispute {
            dispute_id: "d".into(),
            instance: "a".into(),
            opened_at: 1_500,
            status: disputes::Status::Initiated,
            initiator: Some(Initiator::Buyer),
            resolved_at: None,
        }],
        taken: Vec::new(),
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
fn the_summary_names_the_eight_figures_of_the_view() {
    let names: Vec<String> = report(&orders(), &disputes(), WINDOW, NOW)
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(
        names,
        vec![
            "summary.created",
            "summary.completed",
            "summary.completion_rate",
            "summary.volume_sats",
            "summary.active_instances",
            "summary.top_fiat",
            "summary.top_methods",
            "summary.open_disputes",
        ]
    );
}

#[test]
fn the_figures_are_hand_computable_from_the_dataset() {
    // Arrange / Act
    let metrics = report(&orders(), &disputes(), WINDOW, NOW);

    // Assert
    assert_eq!(value(&metrics, "summary.created"), &Value::Count(5));
    assert_eq!(value(&metrics, "summary.completed"), &Value::Count(3));
    assert_eq!(
        value(&metrics, "summary.completion_rate"),
        &Value::Ratio(0.75)
    );
    assert_eq!(value(&metrics, "summary.volume_sats"), &Value::Sats(600));
    assert_eq!(
        value(&metrics, "summary.active_instances"),
        &Value::Count(2)
    );
    assert_eq!(
        value(&metrics, "summary.top_fiat"),
        &Value::Text("ARS (3), VES (2)".into())
    );
    assert_eq!(
        value(&metrics, "summary.top_methods"),
        &Value::Text("cash (4), bank (2), zelle (1)".into())
    );
    assert_eq!(value(&metrics, "summary.open_disputes"), &Value::Count(1));
}

#[test]
fn a_ranking_ties_alphabetically_and_stops_at_top_n() {
    let value = ranking(["b", "a", "d", "c", "a"].into_iter());

    assert_eq!(value, Value::Text("a (2), b (1), c (1)".into()));
}

#[test]
fn an_empty_window_has_nothing_to_rank() {
    let metrics = report(&orders(), &disputes(), Window::new(5_000, 6_000), NOW);

    assert_eq!(value(&metrics, "summary.top_fiat"), &Value::Missing);
    assert_eq!(value(&metrics, "summary.completion_rate"), &Value::Missing);
    assert_eq!(
        value(&metrics, "summary.active_instances"),
        &Value::Count(0)
    );
    assert_eq!(value(&metrics, "summary.volume_sats"), &Value::Sats(0));
}

#[test]
fn every_summary_metric_is_observed() {
    assert!(
        report(&orders(), &disputes(), WINDOW, NOW)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}
