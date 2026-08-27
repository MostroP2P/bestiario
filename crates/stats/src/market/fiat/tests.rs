//! View 5 over a hand-built book (`docs/SPEC.md` §12).

use super::*;
use crate::activity::{Direction, Origin, Status};

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};
const NOW: i64 = 1_900;

fn order(id: &str, instance: &str, fiat: &str, at: i64, sats: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: instance.to_string(),
        created_at: at,
        status: Status::Pending,
        direction: Direction::Buy,
        fiat_code: fiat.into(),
        payment_methods: vec!["cash".into()],
        amount_sats: sats,
        fiat_amount: Some(50.0),
        premium: 2.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(at),
        origin: Origin {
            fiat_code: fiat.into(),
            payment_methods: vec!["cash".into()],
            direction: Direction::Buy,
        },
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: Some(at + 86_400),
    }
}

fn completed(taken_at: i64, success_at: i64, order: Order) -> Order {
    Order {
        status: Status::Success,
        taken_at: Some(taken_at),
        success_at: Some(success_at),
        ..order
    }
}

/// In ARS: Alpha put two on the book (one completed for 10 000 sats, taken
/// 100 s after it was published), Beta one (a sell, completed for 30 000).
/// In USD: one of Alpha's, which this view must not see.
fn book() -> Vec<Order> {
    vec![
        completed(1_100, 1_200, order("a1", "Alpha", "ARS", 1_000, 10_000)),
        order("a2", "Alpha", "ARS", 1_300, 5_000),
        Order {
            direction: Direction::Sell,
            ..completed(1_400, 1_500, order("b1", "Beta", "ARS", 1_050, 30_000))
        },
        completed(1_100, 1_200, order("u1", "Alpha", "USD", 1_000, 99_000)),
    ]
}

fn value<'a>(metrics: &'a [Metric], name: &str) -> &'a Value {
    &metrics
        .iter()
        .find(|metric| metric.name == format!("market.ARS.{name}"))
        .unwrap_or_else(|| panic!("`{name}` is in the view"))
        .value
}

#[test]
fn only_the_orders_standing_in_the_currency_are_in_its_market() {
    // Arrange / Act
    let book = book();
    let in_market = orders_in(&book, "ARS");

    // Assert
    let ids: Vec<&str> = in_market.iter().map(|o| o.order_id.as_str()).collect();
    assert_eq!(ids, ["a1", "a2", "b1"]);
}

#[test]
fn the_view_carries_the_pressure_and_the_premium_of_the_currency() {
    let metrics = report(&book(), "ARS", WINDOW, NOW);

    assert_eq!(value(&metrics, "orders"), &Value::Count(3));
    // Two of the three created are buys.
    assert!(matches!(
        value(&metrics, "buy_orders_share"),
        Value::Ratio(share) if (share - 2.0 / 3.0).abs() < 1e-9
    ));
    // 10 000 of the 40 000 sats completed are buys.
    assert!(matches!(
        value(&metrics, "buy_volume_share"),
        Value::Ratio(share) if (share - 0.25).abs() < 1e-9
    ));
    assert_eq!(value(&metrics, "premium_p50"), &Value::Ratio(0.02));
}

#[test]
fn the_view_carries_how_long_the_book_takes_to_find_a_taker() {
    let metrics = report(&book(), "ARS", WINDOW, NOW);

    // a1 was taken 100 s after it was published, b1 350 s after.
    assert_eq!(value(&metrics, "time_to_fill_samples"), &Value::Count(2));
    assert_eq!(value(&metrics, "time_to_fill_p50"), &Value::Seconds(100));
    assert_eq!(value(&metrics, "time_to_fill_p90"), &Value::Seconds(350));
}

#[test]
fn the_view_names_the_instances_that_trade_the_currency() {
    let metrics = report(&book(), "ARS", WINDOW, NOW);

    assert_eq!(value(&metrics, "instances"), &Value::Count(2));
    assert_eq!(
        value(&metrics, "instances_top3_by_orders"),
        &Value::Text("Alpha 2, Beta 1".into())
    );
    assert_eq!(
        value(&metrics, "instances_top3_by_volume"),
        &Value::Text("Beta 30000 sats, Alpha 10000 sats".into()),
        "by what settled, heaviest first"
    );
}

#[test]
fn a_currency_ranks_no_currencies_and_reports_no_new_ones() {
    let metrics = report(&book(), "ARS", WINDOW, NOW);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert!(!names.iter().any(|name| name.contains("fiat_top3")));
    assert!(!names.iter().any(|name| name.contains("new_fiats")));
    assert!(names.contains(&"market.ARS.method_top3_by_orders"));
}

#[test]
fn a_currency_nobody_traded_is_zeros_and_dashes_not_an_empty_report() {
    let metrics = report(&book(), "XYZ", WINDOW, NOW);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("market.XYZ.{name}"))
            .expect("present")
            .value
    };

    assert!(!metrics.is_empty());
    assert_eq!(value("orders"), &Value::Count(0));
    assert_eq!(value("buy_orders_share"), &Value::Missing);
    assert_eq!(value("instances"), &Value::Count(0));
    assert_eq!(value("instances_top3_by_orders"), &Value::Missing);
    assert_eq!(value("time_to_fill_p50"), &Value::Missing);
}

/// An order first published in ARS and later amended to USD: its market
/// structure belongs to USD, its time-to-fill to ARS.
fn amended() -> Vec<Order> {
    let mut order = completed(1_700, 1_800, order("m1", "Alpha", "USD", 1_100, 7_000));
    order.origin.fiat_code = "ARS".into();
    vec![order]
}

#[test]
fn an_amended_order_is_timed_where_it_entered_the_book() {
    // Arrange / Act
    let ars = report(&amended(), "ARS", WINDOW, NOW);
    let usd = report(&amended(), "USD", WINDOW, NOW);
    let value = |metrics: &[Metric], name: &str| {
        metrics
            .iter()
            .find(|metric| metric.name == format!("market.{name}"))
            .expect("present")
            .value
            .clone()
    };

    // Assert: taken 600 s after it was published, under the currency it
    // waited in, which is what `stats timing --by fiat` would say.
    assert_eq!(value(&ars, "ARS.time_to_fill_p50"), Value::Seconds(600));
    assert_eq!(value(&usd, "USD.time_to_fill_p50"), Value::Missing);
    // Its structure, though, is counted where it now stands.
    assert_eq!(value(&usd, "USD.orders"), Value::Count(1));
    assert_eq!(value(&ars, "ARS.orders"), Value::Count(0));
}

#[test]
fn an_instance_silent_all_window_is_not_ranked_by_orders() {
    // Arrange: Gamma's only ARS order predates the window.
    let mut book = book();
    book.push(order("g1", "Gamma", "ARS", 500, 1_000));

    // Act
    let metrics = report(&book, "ARS", WINDOW, NOW);

    // Assert
    assert_eq!(value(&metrics, "instances"), &Value::Count(2));
    assert_eq!(
        value(&metrics, "instances_top3_by_orders"),
        &Value::Text("Alpha 2, Beta 1".into()),
    );
}

#[test]
fn every_figure_of_the_view_is_observed() {
    assert!(
        report(&book(), "ARS", WINDOW, NOW)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}
