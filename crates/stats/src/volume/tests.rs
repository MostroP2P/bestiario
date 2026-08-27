//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md` §12).

use super::*;

/// These datasets are invented whole, so the archive covers them all.
const ALL: Coverage = Coverage::since(0);
use crate::activity::Origin;

const WINDOW: Window = Window {
    from: 1_000,
    until: 2_000,
};

fn order(id: &str, status: Status, success_at: Option<i64>, sats: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at: 500,
        status,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec![],
        amount_sats: sats,
        fiat_amount: Some(sats as f64 / 100.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: None,
        origin: Origin::default(),
        taken_at: None,
        success_at,
        canceled_at: None,
        expires_at: None,
    }
}

/// Completed in the window: 5k (buy, ARS 50), 30k (sell, ARS 300), 150k
/// (buy, USD 15), 2M (sell, range order: no fiat). Outside: one before, one
/// pending, one canceled.
fn dataset() -> Vec<Order> {
    vec![
        order("a", Status::Success, Some(1_100), 5_000),
        Order {
            direction: Direction::Sell,
            ..order("b", Status::Success, Some(1_200), 30_000)
        },
        Order {
            fiat_code: "USD".into(),
            fiat_amount: Some(15.0),
            ..order("c", Status::Success, Some(1_300), 150_000)
        },
        Order {
            direction: Direction::Sell,
            fiat_amount: None,
            ..order("d", Status::Success, Some(1_900), 2_000_000)
        },
        order("before", Status::Success, Some(999), 999_999),
        order("open", Status::Pending, None, 999_999),
        order("gone", Status::Canceled, None, 999_999),
    ]
}

#[test]
fn volume_sums_the_orders_completed_in_the_window() {
    // Arrange / Act
    let volume = summarise(&dataset(), WINDOW);

    // Assert
    assert_eq!(volume.completed, 4);
    assert_eq!(volume.sats, Some(2_185_000));
    assert_eq!(observed_sats(&dataset(), WINDOW), Some(2_185_000));
}

#[test]
fn tickets_are_average_and_nearest_rank_percentiles_in_sats() {
    let volume = summarise(&dataset(), WINDOW);

    assert_eq!(volume.ticket_avg, Some(546_250));
    assert_eq!(volume.ticket_p50, Some(30_000));
    assert_eq!(volume.ticket_p90, Some(2_000_000));
    assert_eq!(volume.largest, Some(2_000_000));
}

#[test]
fn every_completed_order_lands_in_exactly_one_size_bucket() {
    let volume = summarise(&dataset(), WINDOW);

    // 5k → <10k; 30k → 10k–50k; 150k → 50k–200k; 2M → >1M.
    assert_eq!(volume.buckets, [1, 1, 1, 0, 1]);
    assert_eq!(volume.buckets.iter().sum::<u64>(), volume.completed);
}

#[test]
fn a_named_boundary_belongs_to_the_bucket_it_tops() {
    // 9 999 → <10k; 10 000 and 50 000 → 10k–50k; 1 000 000 → 200k–1M;
    // one sat more → >1M; and the largest order there can be lands too.
    let sizes = [9_999, 10_000, 50_000, 1_000_000, 1_000_001, i64::MAX];
    let orders: Vec<Order> = sizes
        .iter()
        .map(|&size| order(&size.to_string(), Status::Success, Some(1_100), size))
        .collect();

    assert_eq!(summarise(&orders, WINDOW).buckets, [1, 2, 0, 1, 2]);
    assert_eq!(bucket(1_000_000), 3);
    assert_eq!(bucket(1_000_001), 4);
}

#[test]
fn a_sum_beyond_every_satoshi_is_missing_not_wrapped() {
    let orders = vec![
        order("a", Status::Success, Some(1_100), i64::MAX - 1),
        Order {
            direction: Direction::Sell,
            ..order("b", Status::Success, Some(1_100), 2)
        },
    ];

    let volume = summarise(&orders, WINDOW);

    assert_eq!(volume.sats, None, "the total leaves i64");
    assert_eq!(volume.buy_sats, Some(i64::MAX - 1), "each side still fits");
    assert_eq!(volume.sell_sats, Some(2));
    assert_eq!(volume.ticket_avg, Some(i64::MAX / 2 + 1), "the mean fits");
    assert_eq!(volume.completed, 2);
    assert_eq!(observed_sats(&orders, WINDOW), None);
    assert_eq!(
        metrics("volume", &volume)[0].value,
        Value::Missing,
        "reported as missing, not as a wrapped number"
    );
}

#[test]
fn the_average_ticket_is_rounded_to_the_nearest_sat_halves_up() {
    let tickets = |sizes: &[i64]| {
        let orders: Vec<Order> = sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| order(&i.to_string(), Status::Success, Some(1_100), size))
            .collect();
        summarise(&orders, WINDOW).ticket_avg
    };

    assert_eq!(tickets(&[1, 2]), Some(2), "1.5 rounds up");
    assert_eq!(tickets(&[1, 1, 2]), Some(1), "1.33 rounds down");
    assert_eq!(tickets(&[1, 2, 2]), Some(2), "1.67 rounds up");
    assert_eq!(tickets(&[7]), Some(7));
}

#[test]
fn volume_splits_by_the_maker_s_side() {
    let volume = summarise(&dataset(), WINDOW);

    assert_eq!(volume.buy_sats, Some(155_000));
    assert_eq!(volume.sell_sats, Some(2_030_000));
    assert_eq!(
        volume.buy_sats.zip(volume.sell_sats).map(|(b, s)| b + s),
        volume.sats
    );
}

#[test]
fn fiat_volume_is_per_currency_and_skips_range_orders() {
    let volume = summarise(&dataset(), WINDOW);

    let ars = &volume.fiat["ARS"];
    let ars_figures = ars.figures.as_ref().expect("finite");
    assert_eq!(ars_figures.total, 350.0);
    assert_eq!(ars.orders, 2, "the range order has no fiat amount");
    assert_eq!(ars_figures.ticket_avg, 175.0);
    assert_eq!(ars_figures.ticket_p50, 50.0);
    assert_eq!(ars_figures.ticket_p90, 300.0);
    assert_eq!(
        volume.fiat["USD"].figures.as_ref().expect("finite").total,
        15.0
    );
    assert_eq!(volume.fiat.len(), 2);
}

#[test]
fn an_empty_window_is_zero_volume_with_no_tickets() {
    let volume = summarise(&dataset(), Window::new(5_000, 6_000));

    assert_eq!(volume.sats, Some(0));
    assert_eq!(volume.ticket_avg, None);
    assert_eq!(volume.largest, None);
    assert!(volume.fiat.is_empty());
    assert_eq!(
        volume,
        Volume {
            sats: Some(0),
            buy_sats: Some(0),
            sell_sats: Some(0),
            ..Volume::default()
        },
        "nothing completed is zero volume, not an absent one"
    );
}

#[test]
fn no_completed_orders_is_zero_volume_not_a_missing_one() {
    // Unlike a rate, a sum over nothing is a real answer: nothing traded.
    assert_eq!(observed_sats(&[], Window::new(0, 1)), Some(0));
}

#[test]
fn the_global_report_names_the_figures_in_order() {
    let names: Vec<String> = report(&dataset(), WINDOW, None, None, ALL)
        .into_iter()
        .map(|metric| metric.name)
        .collect();

    assert_eq!(
        &names[..13],
        &[
            "volume.sats",
            "volume.completed",
            "volume.ticket_avg",
            "volume.ticket_p50",
            "volume.ticket_p90",
            "volume.largest",
            "volume.size.lt_10k",
            "volume.size.10k_50k",
            "volume.size.50k_200k",
            "volume.size.200k_1m",
            "volume.size.gt_1m",
            "volume.buy_sats",
            "volume.sell_sats",
        ]
    );
    assert_eq!(names[13], "volume.fiat.ARS.total");
    assert_eq!(names.len(), 13 + 2 * 5);
}

#[test]
fn a_fiat_total_carries_its_currency() {
    let metrics = report(&dataset(), WINDOW, None, None, ALL);
    let total = metrics
        .iter()
        .find(|metric| metric.name == "volume.fiat.USD.total")
        .expect("present");

    assert_eq!(
        total.value,
        Value::Fiat {
            amount: 15.0,
            code: "USD".into()
        }
    );
}

#[test]
fn slices_put_the_key_in_the_name() {
    let by_kind = report(&dataset(), WINDOW, Some(Dimension::Kind), None, ALL);
    assert_eq!(by_kind[0].name, "volume.buy.sats");
    assert_eq!(by_kind[0].value, Value::Sats(155_000));

    let by_fiat = report(&dataset(), WINDOW, Some(Dimension::Fiat), None, ALL);
    assert_eq!(by_fiat[0].name, "volume.ARS.sats");

    let by_instance = report(&dataset(), WINDOW, Some(Dimension::Instance), None, ALL);
    assert_eq!(by_instance[0].name, "volume.Alpha (pk).sats");

    // 2026-07-01 to 2026-09-01: two months.
    let by_month = report(
        &dataset(),
        Window::new(1_782_864_000, 1_788_220_800),
        Some(Dimension::Month),
        None,
        ALL,
    );
    assert_eq!(by_month[0].name, "volume.2026-07.sats");
    assert_eq!(by_month.len(), 2 * 13);
}

#[test]
fn every_observed_volume_metric_is_observed() {
    assert!(
        report(&dataset(), WINDOW, None, None, ALL)
            .iter()
            .all(|metric| !metric.is_inferred())
    );
}

#[test]
fn a_fiat_sum_that_overflows_withholds_the_whole_currency_block() {
    // Each amount is finite; their sum is not. Reporting tickets for a
    // currency whose total cannot be stated would be half an answer.
    let orders = vec![
        Order {
            fiat_amount: Some(f64::MAX),
            ..order("a", Status::Success, Some(1_100), 1_000)
        },
        Order {
            fiat_amount: Some(f64::MAX),
            ..order("b", Status::Success, Some(1_100), 1_000)
        },
    ];

    let volume = summarise(&orders, WINDOW);
    let ars = &volume.fiat["ARS"];
    assert_eq!(ars.orders, 2);
    assert_eq!(ars.figures, None);

    let metrics = metrics("volume", &volume);
    let value = |name: &str| {
        &metrics
            .iter()
            .find(|metric| metric.name == format!("volume.fiat.ARS.{name}"))
            .expect("present")
            .value
    };
    assert_eq!(value("orders"), &Value::Count(2));
    for name in ["total", "ticket_avg", "ticket_p50", "ticket_p90"] {
        assert_eq!(value(name), &Value::Missing, "{name}");
    }
}

/// 50k USD/BTC, published at 1000 and again at 1800 so that every order
/// of the dataset settles within five minutes of a snapshot.
fn usd_book() -> crate::rates::RateBook {
    let at = |published_at| crate::rates::Snapshot {
        pubkey: "pk".into(),
        published_at,
        rates: std::collections::BTreeMap::from([("USD".to_string(), 50_000.0)]),
    };
    crate::rates::RateBook::new(vec![at(1_000), at(1_800)])
}

#[test]
fn a_conversion_appends_its_inferred_rows_after_the_observed_ones() {
    // Arrange
    let book = usd_book();
    let conversion = Conversion {
        book: &book,
        code: "USD",
    };

    // Act
    let metrics = report(&dataset(), WINDOW, None, Some(conversion), ALL);

    // Assert: the observed block is untouched, then the four inferred rows.
    let observed = report(&dataset(), WINDOW, None, None, ALL);
    assert_eq!(&metrics[..observed.len()], &observed[..]);
    assert_eq!(metrics.len(), observed.len() + 4);
    let total = &metrics[observed.len()];
    assert_eq!(total.name, "volume.in.USD.total");
    assert!(total.is_inferred());
    // 5k + 30k + 150k + 2M sats at 50k USD/BTC.
    assert_eq!(total.value, Value::fiat(1_092.5, "USD"));
}

#[test]
fn a_conversion_is_reported_once_per_slice() {
    let book = usd_book();
    let conversion = Conversion {
        book: &book,
        code: "USD",
    };

    let by_kind = report(
        &dataset(),
        WINDOW,
        Some(Dimension::Kind),
        Some(conversion),
        ALL,
    );

    let names: Vec<&str> = by_kind
        .iter()
        .map(|metric| metric.name.as_str())
        .filter(|name| name.contains(".in.USD.total"))
        .collect();
    assert_eq!(
        names,
        ["volume.buy.in.USD.total", "volume.sell.in.USD.total"]
    );
    let buy = by_kind
        .iter()
        .find(|metric| metric.name == "volume.buy.in.USD.total")
        .expect("buy total");
    assert_eq!(buy.value, Value::fiat(77.5, "USD"));
}

#[test]
fn a_period_groups_the_window_and_not_the_orders() {
    // The slicing dimensions name a grouping of orders; the periods do
    // not, and `report` answers those before it asks.
    assert_eq!(
        Dimension::Kind.grouping(),
        Some(crate::activity::Dimension::Kind)
    );
    assert_eq!(
        Dimension::Fiat.grouping(),
        Some(crate::activity::Dimension::Fiat)
    );
    assert_eq!(
        Dimension::Instance.grouping(),
        Some(crate::activity::Dimension::Instance)
    );
    assert_eq!(Dimension::Month.grouping(), None);
    assert_eq!(Dimension::Day.grouping(), None);
}
