//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md`
//! §12) for the market structure figures of §6.3.

use super::*;
use crate::activity::Origin;
use crate::activity::Status;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};

fn order(id: &str, direction: Direction, fiat: &str, created_at: i64, sats: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at,
        status: Status::Pending,
        direction,
        fiat_code: fiat.into(),
        payment_methods: vec!["cash".into()],
        amount_sats: sats,
        fiat_amount: Some(1.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(created_at),
        origin: Origin {
            fiat_code: fiat.into(),
            payment_methods: vec!["cash".into()],
            direction,
        },
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: None,
    }
}

/// `order` as published with `methods` on the book.
fn on_book(methods: &[&str], order: Order) -> Order {
    Order {
        origin: Origin {
            payment_methods: methods.iter().map(|m| m.to_string()).collect(),
            ..order.origin.clone()
        },
        ..order
    }
}

fn completed(success_at: i64, premium: f64, order: Order) -> Order {
    Order {
        status: Status::Success,
        success_at: Some(success_at),
        premium,
        ..order
    }
}

/// Created in the window: a (buy ARS 10k, premium 2, completed), b (sell
/// ARS 30k, premium 5, completed), c (buy USD 20k at market price, premium
/// 1, completed, zelle), d (sell USD 40k, range [10, 100], pending), e
/// (buy BRL 5k, canceled, pix). Created before it: `old` (buy ARS 100k,
/// premium 4), completed inside it.
///
/// - pressure: 3 of 5 created are buys → **0.6**; of the 160k sats
///   completed, 130k are buys → **0.8125**
/// - premium over completed [2, 5, 1, 4]: avg **3**, p50 **2**; buys
///   [2, 1, 4] p50 **2**, sells [5] p50 **5**, spread **3**
/// - market price: c of 5 → **0.2**; range: d of 5 → **0.2**, width
///   (100 − 10) / 100 → **0.9**
/// - fiats by orders: ARS 2, USD 2, BRL 1 → HHI **0.36**; by volume: ARS
///   140k, USD 20k → HHI **0.78125**
/// - methods by orders: cash 3, pix 1, zelle 1; by volume: cash 140k,
///   zelle 20k
/// - first seen in the window: BRL, USD; pix, zelle
fn dataset() -> Vec<Order> {
    vec![
        completed(1_100, 2.0, order("a", Direction::Buy, "ARS", 1_050, 10_000)),
        completed(
            1_200,
            5.0,
            order("b", Direction::Sell, "ARS", 1_060, 30_000),
        ),
        on_book(
            &["zelle"],
            Order {
                is_market_price: true,
                payment_methods: vec!["zelle".into()],
                ..completed(1_300, 1.0, order("c", Direction::Buy, "USD", 1_250, 20_000))
            },
        ),
        Order {
            fiat_range: Some((10.0, 100.0)),
            fiat_amount: None,
            ..order("d", Direction::Sell, "USD", 1_300, 40_000)
        },
        on_book(
            &["pix"],
            Order {
                status: Status::Canceled,
                canceled_at: Some(1_450),
                payment_methods: vec!["pix".into()],
                ..order("e", Direction::Buy, "BRL", 1_400, 5_000)
            },
        ),
        completed(
            1_500,
            4.0,
            order("old", Direction::Buy, "ARS", 500, 100_000),
        ),
    ]
}

fn approx(actual: Option<f64>, expected: f64) -> bool {
    actual.is_some_and(|actual| (actual - expected).abs() < 1e-9)
}

#[test]
fn pressure_is_the_buy_share_of_orders_created_and_of_sats_completed() {
    // Arrange / Act
    let market = summarise(&dataset(), WINDOW);

    // Assert
    assert_eq!(market.orders, 5);
    assert!(approx(market.buy_orders_share, 0.6));
    assert!(approx(market.buy_volume_share, 0.8125));
}

#[test]
fn premium_is_over_the_orders_completed_in_the_window() {
    let market = summarise(&dataset(), WINDOW);

    assert!(approx(market.premium_avg, 3.0));
    assert!(approx(market.premium_p50, 2.0));
    assert!(approx(market.premium_p50_buy, 2.0));
    assert!(approx(market.premium_p50_sell, 5.0));
    assert!(approx(market.premium_spread, 3.0));
}

#[test]
fn market_price_and_range_shares_are_over_the_orders_created() {
    let market = summarise(&dataset(), WINDOW);

    assert!(approx(market.market_price_share, 0.2));
    assert!(approx(market.range_share, 0.2));
    assert!(approx(market.range_width_avg, 0.9));
}

#[test]
fn the_fiat_ranking_is_by_orders_created_and_by_sats_completed() {
    let market = summarise(&dataset(), WINDOW);

    assert_eq!(
        market.fiats_by_orders.entries,
        vec![
            ("ARS".to_string(), 2),
            ("USD".to_string(), 2),
            ("BRL".to_string(), 1)
        ]
    );
    assert!(approx(Some(market.fiats_by_orders.top3_share), 1.0));
    assert!(approx(Some(market.fiats_by_orders.hhi), 0.36));
    assert_eq!(
        market.fiats_by_volume.entries,
        vec![("ARS".to_string(), 140_000), ("USD".to_string(), 20_000)]
    );
    assert!(approx(Some(market.fiats_by_volume.hhi), 0.78125));
}

#[test]
fn the_method_ranking_counts_every_method_an_order_names() {
    let market = summarise(&dataset(), WINDOW);

    assert_eq!(
        market.methods_by_orders.entries,
        vec![
            ("cash".to_string(), 3),
            ("pix".to_string(), 1),
            ("zelle".to_string(), 1)
        ]
    );
    assert_eq!(
        market.methods_by_volume.entries,
        vec![("cash".to_string(), 140_000), ("zelle".to_string(), 20_000)]
    );
}

#[test]
fn first_sightings_are_what_had_never_been_seen_before_the_window() {
    let market = summarise(&dataset(), WINDOW);

    assert_eq!(market.new_fiats, vec!["BRL", "USD"]);
    assert_eq!(market.new_methods, vec!["pix", "zelle"]);
}

#[test]
fn a_ranking_over_more_than_three_keeps_the_rest_in_the_shares() {
    // Four fiats with one order each: top-3 concentration is 3/4.
    let orders: Vec<Order> = ["ARS", "BRL", "CUP", "EUR"]
        .iter()
        .enumerate()
        .map(|(i, fiat)| order(fiat, Direction::Buy, fiat, 1_100 + i as i64, 1_000))
        .collect();

    let market = summarise(&orders, WINDOW);

    assert!(approx(Some(market.fiats_by_orders.top3_share), 0.75));
    assert!(approx(Some(market.fiats_by_orders.hhi), 0.25));
    assert_eq!(market.fiats_by_orders.entries.len(), 4);
}

#[test]
fn an_empty_window_has_no_shares_and_empty_rankings() {
    let market = summarise(&dataset(), Window::new(5_000, 6_000));

    assert_eq!(market.orders, 0);
    assert_eq!(market.buy_orders_share, None);
    assert_eq!(market.premium_p50, None);
    assert_eq!(market.range_width_avg, None);
    assert!(market.fiats_by_orders.entries.is_empty());
    assert!(market.new_fiats.is_empty());
}

#[test]
fn the_global_report_names_the_figures_in_order_all_observed() {
    let metrics = report(&dataset(), WINDOW, None);
    let names: Vec<&str> = metrics.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(
        names,
        [
            "market.orders",
            "market.buy_orders_share",
            "market.buy_volume_share",
            "market.premium_avg",
            "market.premium_p50",
            "market.premium_p50_buy",
            "market.premium_p50_sell",
            "market.premium_spread",
            "market.market_price_share",
            "market.range_share",
            "market.range_width_avg",
            "market.fiat_top3_by_orders",
            "market.fiat_top3_orders_share",
            "market.fiat_hhi_orders",
            "market.fiat_top3_by_volume",
            "market.fiat_top3_volume_share",
            "market.fiat_hhi_volume",
            "market.method_top3_by_orders",
            "market.method_top3_by_volume",
            "market.new_fiats",
            "market.new_methods",
        ]
    );
    assert!(metrics.iter().all(|metric| !metric.is_inferred()));
}

#[test]
fn premiums_are_percent_figures_and_rankings_are_text() {
    let metrics = report(&dataset(), WINDOW, None);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("market.{name}"))
            .expect("present")
            .value
    };

    // A 3% premium is the ratio 0.03, rendered as a percentage.
    assert_eq!(value("premium_avg"), &Value::Ratio(0.03));
    assert_eq!(value("premium_spread"), &Value::Ratio(0.03));
    assert_eq!(
        value("fiat_top3_by_orders"),
        &Value::Text("ARS 2, USD 2, BRL 1".into())
    );
    assert_eq!(
        value("fiat_top3_by_volume"),
        &Value::Text("ARS 140000 sats, USD 20000 sats".into())
    );
    assert_eq!(value("new_fiats"), &Value::Text("BRL, USD".into()));
    assert!(matches!(value("fiat_hhi_orders"), Value::Ratio(hhi) if (hhi - 0.36).abs() < 1e-9));
}

#[test]
fn nothing_new_and_nothing_ranked_are_missing_not_empty_text() {
    let metrics = report(&dataset(), Window::new(5_000, 6_000), None);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("market.{name}"))
            .expect("present")
            .value
    };

    assert_eq!(value("new_fiats"), &Value::Missing);
    assert_eq!(value("fiat_top3_by_orders"), &Value::Missing);
    assert_eq!(value("fiat_hhi_orders"), &Value::Missing);
}

#[test]
fn a_fiat_slice_drops_the_fiat_ranking_and_keeps_the_rest() {
    let by_fiat = report(&dataset(), WINDOW, Some(Dimension::Fiat));
    let names: Vec<&str> = by_fiat.iter().map(|metric| metric.name.as_str()).collect();

    assert_eq!(names[0], "market.ARS.orders");
    assert!(names.contains(&"market.ARS.premium_spread"));
    assert!(names.contains(&"market.ARS.method_top3_by_orders"));
    assert!(!names.iter().any(|name| name.contains("fiat_top3")));
    assert!(!names.iter().any(|name| name.contains("fiat_hhi")));
    assert!(!names.iter().any(|name| name.contains("new_fiats")));
    // Per block: 21 − 6 ranking rows − 1 new_fiats row; USD alone also
    // carries the absolute range width, which needs a single currency.
    assert_eq!(by_fiat.len(), 3 * 14 + 1);
    assert!(names.contains(&"market.USD.range_width_fiat_avg"));
    assert!(!names.contains(&"market.ARS.range_width_fiat_avg"));
}

#[test]
fn kind_and_instance_slices_keep_every_row() {
    let by_kind = report(&dataset(), WINDOW, Some(Dimension::Kind));
    assert_eq!(by_kind[0].name, "market.buy.orders");
    assert_eq!(by_kind.len(), 2 * 21);

    let by_instance = report(&dataset(), WINDOW, Some(Dimension::Instance));
    assert_eq!(by_instance[0].name, "market.Alpha (pk).orders");
}

#[test]
fn a_long_list_names_the_first_few_and_counts_the_rest() {
    // Ten methods first seen in the window, one order each.
    let orders: Vec<Order> = (0..10)
        .map(|i| {
            on_book(
                &[&format!("method-{i:02}")],
                order(&format!("o{i}"), Direction::Buy, "ARS", 1_100 + i, 1_000),
            )
        })
        .collect();

    let metrics = report(&orders, WINDOW, None);
    let new_methods = metrics
        .iter()
        .find(|metric| metric.name == "market.new_methods")
        .expect("present");

    assert_eq!(
        new_methods.value,
        Value::Text(
            "method-00, method-01, method-02, method-03, method-04, method-05, method-06, \
             method-07, +2 more"
                .into()
        )
    );
}

#[test]
fn the_method_ranking_counts_what_was_on_the_book_not_a_later_amendment() {
    // Arrange: an order created with `cash` that later advertises `zelle`
    // too. The book held `cash` alone.
    let amended = Order {
        payment_methods: vec!["cash".into(), "zelle".into()],
        ..order("amended", Direction::Buy, "ARS", 1_100, 10_000)
    };

    // Act
    let market = summarise(&[amended], WINDOW);

    // Assert
    assert_eq!(
        market.methods_by_orders.entries,
        vec![("cash".to_string(), 1)]
    );
}

#[test]
fn a_method_added_to_an_old_order_does_not_backdate_its_first_sighting() {
    // Arrange: an order from before the window that now advertises `pix`,
    // and an order created inside it that was published with `pix`. Dating
    // `pix` by the old order would hide a genuine first sighting.
    let old = Order {
        payment_methods: vec!["cash".into(), "pix".into()],
        ..order("old", Direction::Buy, "ARS", 500, 10_000)
    };
    let fresh = on_book(
        &["pix"],
        Order {
            payment_methods: vec!["pix".into()],
            ..order("fresh", Direction::Buy, "ARS", 1_100, 10_000)
        },
    );

    // Act
    let market = summarise(&[old, fresh], WINDOW);

    // Assert
    assert_eq!(market.new_methods, vec!["pix"]);
}

#[test]
fn the_range_width_is_relative_everywhere_and_absolute_only_within_one_fiat() {
    // Arrange: ARS [900, 1000] is the narrow one in its own currency and
    // the wide one relative to its top; USD [10, 100] is the reverse.
    let orders = vec![
        Order {
            fiat_range: Some((900.0, 1_000.0)),
            fiat_amount: None,
            ..order("ars", Direction::Sell, "ARS", 1_100, 10_000)
        },
        Order {
            fiat_range: Some((10.0, 100.0)),
            fiat_amount: None,
            ..order("usd", Direction::Sell, "USD", 1_200, 10_000)
        },
    ];

    // Act
    let market = summarise(&orders, WINDOW);
    let global = metrics("market", &market, None);
    let by_fiat = report(&orders, WINDOW, Some(Dimension::Fiat));
    let named = |name: &str| {
        by_fiat
            .iter()
            .find(|metric| metric.name == name)
            .map(|metric| metric.value.clone())
    };

    // Assert: the mean of 0.10 and 0.90 across currencies, and each
    // currency's own width beside it in the slice.
    assert!(approx(market.range_width_avg, 0.5));
    assert!(approx(market.range_width_fiat_avg, 95.0));
    assert!(
        !global
            .iter()
            .any(|metric| metric.name.ends_with("range_width_fiat_avg")),
        "a block that mixes currencies cannot average their widths"
    );
    assert_eq!(
        named("market.ARS.range_width_fiat_avg"),
        Some(Value::fiat(100.0, "ARS"))
    );
    assert_eq!(
        named("market.USD.range_width_fiat_avg"),
        Some(Value::fiat(90.0, "USD"))
    );
}

#[test]
fn a_book_of_i64_max_orders_still_reports_a_share() {
    // `amt` is only checked for being non-negative, so an instance can
    // publish `i64::MAX` sats. Two of them summed in `i64` panic in debug
    // and wrap negative in release, which would report the buy share of a
    // 50/50 book as missing.
    let buy = completed(
        1_100,
        1.0,
        order("a", Direction::Buy, "ARS", 1_050, i64::MAX),
    );
    let sell = completed(
        1_200,
        1.0,
        order("b", Direction::Sell, "ARS", 1_060, i64::MAX),
    );

    let market = summarise(&[buy, sell], WINDOW);

    assert!(approx(market.buy_volume_share, 0.5));
    assert_eq!(
        market.fiats_by_volume.entries,
        vec![("ARS".to_string(), 2 * i128::from(i64::MAX))]
    );
}

#[test]
fn a_zero_ended_range_counts_as_a_range_and_not_in_the_widths() {
    // `fa = [0, 0]` passes the parser — a fiat amount is only checked for
    // being finite and non-negative — and has no relative width to take.
    let orders = vec![
        Order {
            fiat_range: Some((0.0, 0.0)),
            ..order("zero", Direction::Buy, "ARS", 1_100, 1_000)
        },
        Order {
            fiat_range: Some((10.0, 100.0)),
            ..order("wide", Direction::Buy, "ARS", 1_200, 1_000)
        },
    ];

    let market = summarise(&orders, WINDOW);

    // Both are ranges; only one has a width.
    assert!(approx(market.range_share, 1.0));
    assert!(approx(market.range_width_avg, 0.9));
    assert!(approx(market.range_width_fiat_avg, 90.0));
}

#[test]
fn a_slice_dates_its_first_sightings_against_its_own_orders() {
    // `pix` has been on the book as a sell since before the window; a
    // buyer names it for the first time inside it. Globally that is not a
    // first sighting; in the `buy` block it is.
    let orders = vec![
        on_book(
            &["pix"],
            Order {
                payment_methods: vec!["pix".into()],
                ..order("old-sell", Direction::Sell, "BRL", 500, 1_000)
            },
        ),
        on_book(
            &["pix"],
            Order {
                payment_methods: vec!["pix".into()],
                ..order("new-buy", Direction::Buy, "BRL", 1_100, 1_000)
            },
        ),
    ];

    assert!(summarise(&orders, WINDOW).new_methods.is_empty());

    let by_kind = report(&orders, WINDOW, Some(Dimension::Kind));
    let value = |name: &str| {
        &by_kind
            .iter()
            .find(|metric| metric.name == name)
            .expect("present")
            .value
    };

    assert_eq!(value("market.buy.new_methods"), &Value::Text("pix".into()));
    assert_eq!(value("market.sell.new_methods"), &Value::Missing);
}

#[test]
fn an_order_is_credited_to_every_method_it_names() {
    // One completed order of 1 361 sats offered over two methods shows
    // 1 361 against each: the sats are attributed, not split, so the
    // method ranking adds up to more than the volume traded.
    let orders = vec![on_book(
        &["Efectivo", "EnZona"],
        Order {
            payment_methods: vec!["Efectivo".into(), "EnZona".into()],
            ..completed(
                1_100,
                0.0,
                order("one", Direction::Sell, "CUP", 1_050, 1_361),
            )
        },
    )];

    let market = summarise(&orders, WINDOW);

    assert_eq!(
        market.methods_by_volume.entries,
        vec![
            ("Efectivo".to_string(), 1_361),
            ("EnZona".to_string(), 1_361)
        ]
    );
    // The currency ranking, which an order contributes to exactly once,
    // still adds up to the volume.
    assert_eq!(
        market.fiats_by_volume.entries,
        vec![("CUP".to_string(), 1_361)]
    );
}
