//! A hand-built dataset and hand-computed expected values (`docs/SPEC.md`
//! §12) for a metric plotted over time.

use super::*;
use crate::activity::{Direction, Origin, Status};
use crate::dev_fees::Fee;
use crate::rates::Snapshot;

/// 2026-07-01, 2026-08-01 and 2026-09-01 at midnight UTC.
const JULY: i64 = 1_782_864_000;
const AUGUST: i64 = 1_785_542_400;
const SEPTEMBER: i64 = 1_788_220_800;
const NOW: i64 = SEPTEMBER;
const DAY: i64 = 86_400;

fn order(id: &str, created_at: i64, fiat: &str) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at,
        status: Status::Pending,
        direction: Direction::Buy,
        fiat_code: fiat.into(),
        payment_methods: vec!["cash".into()],
        amount_sats: 10_000,
        fiat_amount: Some(50.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(created_at),
        origin: Origin {
            fiat_code: fiat.into(),
            payment_methods: vec!["cash".into()],
            direction: Direction::Buy,
        },
        taken_at: None,
        success_at: None,
        canceled_at: None,
        expires_at: Some(created_at + DAY),
    }
}

fn completed(created_at: i64, success_at: i64, sats: i64, fiat: &str, id: &str) -> Order {
    Order {
        status: Status::Success,
        taken_at: Some(created_at + 60),
        success_at: Some(success_at),
        amount_sats: sats,
        ..order(id, created_at, fiat)
    }
}

/// One order created and completed in July (5 000 sats, ARS), two in
/// August (10 000 and 30 000 sats, ARS and USD).
fn data() -> Data {
    Data {
        orders: vec![
            completed(JULY + DAY, JULY + 2 * DAY, 5_000, "ARS", "j1"),
            completed(AUGUST + DAY, AUGUST + 2 * DAY, 10_000, "ARS", "a1"),
            completed(AUGUST + 3 * DAY, AUGUST + 4 * DAY, 30_000, "USD", "a2"),
            order("open", AUGUST + 5 * DAY, "ARS"),
        ],
        ..Data::default()
    }
}

fn window() -> Window {
    Window::new(JULY, SEPTEMBER)
}

fn named<'a>(metrics: &'a [Metric], name: &str) -> &'a Value {
    &metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the report"))
        .value
}

#[test]
fn a_metric_is_evaluated_once_per_bucket() {
    // Arrange / Act
    let metrics =
        report(&data(), window(), Period::Month, "volume.sats", None, NOW).expect("a series");

    // Assert
    assert_eq!(named(&metrics, "volume.sats.2026-07"), &Value::Sats(5_000));
    assert_eq!(named(&metrics, "volume.sats.2026-08"), &Value::Sats(40_000));
    assert_eq!(metrics.len(), 2 * 2, "a value and a delta per bucket");
}

#[test]
fn the_first_bucket_has_nothing_to_have_changed_from() {
    let metrics =
        report(&data(), window(), Period::Month, "volume.sats", None, NOW).expect("a series");

    assert_eq!(
        named(&metrics, "volume.sats.2026-07.delta"),
        &Value::Missing
    );
    // 5 000 → 40 000 is seven times as much.
    assert!(matches!(
        named(&metrics, "volume.sats.2026-08.delta"),
        Value::Ratio(delta) if (delta - 7.0).abs() < 1e-12
    ));
}

#[test]
fn a_proportion_changes_by_points_not_by_a_proportion_of_itself() {
    // A rate that goes from 100% to 50% fell by fifty points.
    let before = Value::Ratio(1.0);
    let after = Value::Ratio(0.5);

    assert_eq!(delta(&before, &after), Value::Ratio(-0.5));
    // While a magnitude that halves fell by half.
    assert_eq!(
        delta(&Value::Count(10), &Value::Count(5)),
        Value::Ratio(-0.5)
    );
    assert_eq!(delta(&Value::Sats(10), &Value::Sats(30)), Value::Ratio(2.0));
}

#[test]
fn a_change_from_nothing_is_not_a_proportion() {
    assert_eq!(delta(&Value::Count(0), &Value::Count(5)), Value::Missing);
    assert_eq!(delta(&Value::Missing, &Value::Count(5)), Value::Missing);
    assert_eq!(
        delta(&Value::Text("x".into()), &Value::Count(5)),
        Value::Missing
    );
}

#[test]
fn every_family_of_the_spec_can_be_plotted() {
    for metric in [
        "orders.created",
        "volume.sats",
        "dev_fees.total_sats",
        "disputes.opened",
    ] {
        let series = report(&data(), window(), Period::Month, metric, None, NOW)
            .unwrap_or_else(|error| panic!("{metric}: {error}"));
        assert_eq!(series.len(), 4, "{metric}");
    }
}

#[test]
fn the_bucket_size_is_what_was_asked_for() {
    let by_day = report(
        &data(),
        Window::new(AUGUST, AUGUST + 3 * DAY),
        Period::Day,
        "orders.created",
        None,
        NOW,
    )
    .expect("a series");

    let names: Vec<&str> = by_day
        .iter()
        .map(|metric| metric.name.as_str())
        .filter(|name| !name.ends_with(".delta"))
        .collect();
    assert_eq!(
        names,
        [
            "orders.created.2026-08-01",
            "orders.created.2026-08-02",
            "orders.created.2026-08-03"
        ]
    );
}

#[test]
fn a_split_plots_one_line_per_slice() {
    let metrics = report(
        &data(),
        window(),
        Period::Month,
        "volume.sats",
        Some(Split::Fiat),
        NOW,
    )
    .expect("a series");

    assert_eq!(
        named(&metrics, "volume.sats.ARS.2026-07"),
        &Value::Sats(5_000)
    );
    assert_eq!(
        named(&metrics, "volume.sats.ARS.2026-08"),
        &Value::Sats(10_000)
    );
    assert_eq!(named(&metrics, "volume.sats.USD.2026-07"), &Value::Sats(0));
    assert_eq!(
        named(&metrics, "volume.sats.USD.2026-08"),
        &Value::Sats(30_000)
    );
}

#[test]
fn a_family_whose_events_carry_no_such_tag_is_not_split_by_it() {
    let error = report(
        &data(),
        window(),
        Period::Month,
        "dev_fees.total_sats",
        Some(Split::Fiat),
        NOW,
    )
    .expect_err("refused");

    assert_eq!(
        error,
        SeriesError::CannotSplit {
            metric: "dev_fees.total_sats".to_string(),
            split: Split::Fiat,
        }
    );
    // By instance it is fine: a fee names one.
    assert!(
        report(
            &data(),
            window(),
            Period::Month,
            "dev_fees.total_sats",
            Some(Split::Instance),
            NOW
        )
        .is_ok()
    );
}

#[test]
fn a_figure_about_now_has_no_shape_over_time() {
    for metric in ["orders.open_now", "disputes.open_now"] {
        let error =
            report(&data(), window(), Period::Month, metric, None, NOW).expect_err("refused");
        assert!(
            matches!(error, SeriesError::AboutNow { .. }),
            "{metric}: {error}"
        );
    }
}

#[test]
fn a_delta_is_not_plotted_against_itself() {
    let error = report(
        &data(),
        window(),
        Period::Month,
        "orders.created_delta",
        None,
        NOW,
    )
    .expect_err("refused");

    assert!(matches!(error, SeriesError::AlreadyADelta { .. }));
}

#[test]
fn a_name_no_family_reports_is_unknown() {
    for metric in ["orders.nonsense", "nonsense.created", ""] {
        let error =
            report(&data(), window(), Period::Month, metric, None, NOW).expect_err("refused");
        assert!(
            matches!(error, SeriesError::UnknownMetric { .. }),
            "{metric}: {error}"
        );
    }
}

#[test]
fn a_series_nobody_could_read_is_refused_before_it_is_computed() {
    let long = Window::new(0, (MAX_BUCKETS as i64 + 2) * DAY);

    let error =
        report(&data(), long, Period::Day, "orders.created", None, NOW).expect_err("refused");

    assert!(matches!(error, SeriesError::TooManyBuckets));
    // The same range by month is a table anybody can read.
    assert!(report(&data(), long, Period::Month, "orders.created", None, NOW).is_ok());
}

#[test]
fn without_the_assumption_the_inferred_rows_are_not_offered() {
    // They rest on a `dev_fee_percentage` this run was not given.
    assert!(
        !catalogue(&data(), window(), NOW).contains(&"dev_fees.implied_volume".to_string()),
        "nothing supports the figure"
    );
    let with_assumption = Data {
        dev_fee_pct: Some(Assumption {
            per_instance: BTreeMap::new(),
            default: 0.30,
        }),
        ..data()
    };
    assert!(
        catalogue(&with_assumption, window(), NOW).contains(&"dev_fees.implied_volume".to_string())
    );
}

#[test]
fn the_catalogue_is_what_the_families_report_minus_what_cannot_be_plotted() {
    let catalogue = catalogue(&data(), window(), NOW);

    assert!(catalogue.contains(&"orders.created".to_string()));
    assert!(catalogue.contains(&"volume.sats".to_string()));
    assert!(catalogue.contains(&"dev_fees.coverage".to_string()));
    assert!(catalogue.contains(&"disputes.opened".to_string()));
    assert!(!catalogue.iter().any(|name| name.ends_with("_now")));
    assert!(!catalogue.iter().any(|name| name.ends_with("_delta")));
    // Every name it offers is one `report` accepts.
    for metric in &catalogue {
        assert!(
            report(&data(), window(), Period::Month, metric, None, NOW).is_ok(),
            "{metric}"
        );
    }
}

#[test]
fn a_metric_the_family_reports_is_plottable_without_a_line_changing_here() {
    // The registry is the family's own block: a metric it reports is one a
    // series can ask for. `dev_fees.implied_volume` arrived in phase 3 and
    // nothing here was told about it.
    let fees = DevFeeData {
        fees: vec![Fee {
            event_id: "f".into(),
            order_id: "a1".into(),
            pubkey: "pk".into(),
            instance: "Alpha (pk)".into(),
            created_at: AUGUST + 2 * DAY,
            amount_sats: 60,
            is_duplicate: false,
            order_known: true,
            settled_at: Some(AUGUST + 2 * DAY),
            fee_in_force: Some(0.006),
            settled_amount_sats: Some(10_000),
        }],
        settlements: vec![],
    };
    let data = Data {
        fees,
        dev_fee_pct: Some(Assumption {
            per_instance: BTreeMap::new(),
            default: 0.30,
        }),
        ..data()
    };

    let series = report(
        &data,
        window(),
        Period::Month,
        "dev_fees.implied_volume",
        None,
        NOW,
    )
    .expect("a series");

    assert_eq!(
        named(&series, "dev_fees.implied_volume.2026-07"),
        &Value::Sats(0)
    );
    assert!(matches!(
        named(&series, "dev_fees.implied_volume.2026-08"),
        Value::Sats(sats) if *sats > 0
    ));
}

/// The whole metric, not just its number — what provenance is carried on.
fn metric<'a>(metrics: &'a [Metric], name: &str) -> &'a Metric {
    metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the report"))
}

/// The dev fee data the inferred §6.6 rows are computed from: one fee, paid
/// in August against a 10 000 sat order.
fn with_a_fee() -> Data {
    Data {
        fees: DevFeeData {
            fees: vec![Fee {
                event_id: "f".into(),
                order_id: "a1".into(),
                pubkey: "pk".into(),
                instance: "Alpha (pk)".into(),
                created_at: AUGUST + 2 * DAY,
                amount_sats: 60,
                is_duplicate: false,
                order_known: true,
                settled_at: Some(AUGUST + 2 * DAY),
                fee_in_force: Some(0.006),
                settled_amount_sats: Some(10_000),
            }],
            settlements: vec![],
        },
        dev_fee_pct: Some(Assumption {
            per_instance: BTreeMap::new(),
            default: 0.30,
        }),
        ..data()
    }
}

#[test]
fn an_inferred_figure_is_still_inferred_once_it_is_a_bucket() {
    // `dev_fees.implied_volume` rests on an assumed fee share. A bucket of
    // it renamed into an observation would print without `(inf)` and
    // serialise as a measurement — the one thing §5 says a figure may not
    // do.
    let series = report(
        &with_a_fee(),
        window(),
        Period::Month,
        "dev_fees.implied_volume",
        None,
        NOW,
    )
    .expect("a series");

    let july = metric(&series, "dev_fees.implied_volume.2026-07");
    assert!(july.is_inferred(), "the bucket keeps what qualifies it");
    assert!(july.error().is_some());

    // And so does the Δ: a change between two estimates is an estimate,
    // qualified by what qualifies the bucket it is the change *to*.
    let august = metric(&series, "dev_fees.implied_volume.2026-08");
    let delta = metric(&series, "dev_fees.implied_volume.2026-08.delta");
    assert!(delta.is_inferred());
    assert_eq!(delta.error(), august.error());
}

#[test]
fn an_observed_figure_is_not_dressed_up_as_an_inferred_one() {
    // The other direction of the same rule: nothing here adds a
    // qualification a family did not give.
    let series = report(
        &with_a_fee(),
        window(),
        Period::Month,
        "volume.sats",
        None,
        NOW,
    )
    .expect("a series");

    assert!(series.iter().all(|metric| !metric.is_inferred()));
    assert!(series.iter().all(|metric| metric.error().is_none()));
}

#[test]
fn a_metric_the_archive_itself_names_is_not_unknown() {
    // `volume.fiat.ARS.total` exists because an ARS order completed. Judged
    // against a window nothing completed in, every per-currency and
    // per-instance metric of a real archive would be refused as unknown.
    let series = report(
        &data(),
        window(),
        Period::Month,
        "volume.fiat.ARS.total",
        None,
        NOW,
    )
    .expect("ARS completed inside this window");

    assert_eq!(
        named(&series, "volume.fiat.ARS.total.2026-07"),
        &Value::Fiat {
            amount: 50.0,
            code: "ARS".to_string()
        }
    );
    assert!(catalogue(&data(), window(), NOW).contains(&"volume.fiat.ARS.total".to_string()));
}

#[test]
fn a_currency_nothing_completed_in_is_still_unknown() {
    // The rule has to cut both ways, or "unknown metric" would mean
    // nothing: JPY is not in this archive, and a plot of it would be a
    // column of zeros presented as an answer.
    let error = report(
        &data(),
        window(),
        Period::Month,
        "volume.fiat.JPY.total",
        None,
        NOW,
    )
    .expect_err("nothing completed in JPY");

    assert!(
        matches!(error, SeriesError::UnknownMetric { .. }),
        "{error}"
    );
}

#[test]
fn the_currency_of_a_converted_metric_is_read_off_its_name() {
    assert_eq!(priced_in("volume.in.USD.total").as_deref(), Some("USD"));
    assert_eq!(priced_in("volume.in.ARS.orders").as_deref(), Some("ARS"));
    // Not a currency code, and not a converted metric at all.
    assert_eq!(priced_in("volume.in.usd.total"), None);
    assert_eq!(priced_in("volume.in.DOLLARS.total"), None);
    assert_eq!(priced_in("volume.sats"), None);
    assert_eq!(priced_in("orders.created"), None);
}

#[test]
fn a_converted_volume_is_a_series_like_any_other_6_2_metric() {
    // §6.2's converted rows are what `stats volume --in USD` prints, and
    // the roadmap asks for *any* §6.2 metric over time. Without a book to
    // price from, the name would be refused as unknown.
    let priced = Data {
        priced: Some(Priced {
            book: RateBook::new(vec![
                Snapshot {
                    pubkey: "pk".into(),
                    published_at: JULY + 2 * DAY,
                    rates: BTreeMap::from([("USD".to_string(), 100_000_000.0)]),
                },
                Snapshot {
                    pubkey: "pk".into(),
                    published_at: AUGUST + 2 * DAY,
                    rates: BTreeMap::from([("USD".to_string(), 100_000_000.0)]),
                },
            ]),
            code: "USD".to_string(),
        }),
        ..data()
    };

    let series = report(
        &priced,
        window(),
        Period::Month,
        "volume.in.USD.total",
        None,
        NOW,
    )
    .expect("a series of a converted figure");

    // 5 000 sats is 0.00005 BTC, and a BTC is 100 000 000 USD here: 5 000.
    assert_eq!(
        named(&series, "volume.in.USD.total.2026-07"),
        &Value::Fiat {
            amount: 5_000.0,
            code: "USD".to_string()
        }
    );
    assert!(
        metric(&series, "volume.in.USD.total.2026-07").is_inferred(),
        "a converted figure rests on a published rate"
    );
    assert!(catalogue(&priced, window(), NOW).contains(&"volume.in.USD.total".to_string()));
}

#[test]
fn a_range_nobody_could_bucket_is_refused_without_being_built() {
    // Every second since the epoch, by day: a hundred trillion buckets,
    // and a cursor that walks past the last year `chrono` can represent on
    // the way to counting them. The refusal has to come first.
    let everything = Window::new(0, i64::MAX);

    let error = report(
        &data(),
        everything,
        Period::Day,
        "orders.created",
        None,
        NOW,
    )
    .expect_err("refused");

    assert!(matches!(error, SeriesError::TooManyBuckets), "{error}");
    // And by year, where the same range is still past the limit, rather
    // than a panic at the end of the calendar.
    assert!(matches!(
        report(
            &data(),
            everything,
            Period::Year,
            "orders.created",
            None,
            NOW
        ),
        Err(SeriesError::TooManyBuckets)
    ));
}
