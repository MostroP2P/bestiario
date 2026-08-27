use super::*;

#[test]
fn an_observed_metric_carries_no_error() {
    // Arrange / Act
    let metric = Metric::observed("orders.created", Value::Count(42));

    // Assert
    assert_eq!(metric.kind(), MetricKind::Observed);
    assert_eq!(metric.error(), None);
    assert!(!metric.is_inferred());
}

#[test]
fn an_inferred_metric_cannot_be_built_without_its_error() {
    // The type is the enforcement: there is no constructor that produces an
    // inferred metric with `error: None`, so §5's "reported with its error
    // column" cannot be skipped by forgetting.
    let metric = Metric::inferred(
        "volume.from_dev_fee",
        Value::Sats(1_000),
        "±1 sat amplified by 1/(fee × pct)",
    );

    assert!(metric.is_inferred());
    assert_eq!(metric.error(), Some("±1 sat amplified by 1/(fee × pct)"));
}

#[test]
fn a_value_serializes_with_the_unit_that_makes_it_readable() {
    // A bare number says nothing about whether 0.5 is half of something or
    // fifty basis points. The unit travels with it.
    let json = serde_json::to_value(Value::Ratio(0.5)).expect("serialize");

    assert_eq!(json, serde_json::json!({ "unit": "ratio", "value": 0.5 }));
}

#[test]
fn nothing_to_report_is_not_zero() {
    let json = serde_json::to_value(Value::Missing).expect("serialize");

    // `value` is present and null rather than absent: §10 gives every
    // metric record a `value` member, and a consumer indexing it should find
    // "nothing" rather than a structurally different object.
    assert_eq!(
        json,
        serde_json::json!({ "unit": "missing", "value": null })
    );
}

#[test]
fn a_ratio_is_any_finite_number() {
    // Growth against the previous period is a ratio too, and it can be
    // negative or exceed one; only finiteness is enforced.
    for ratio in [0.0, 1.0, 2.0, -0.5] {
        assert_eq!(Value::ratio(ratio), Value::Ratio(ratio));
        assert_eq!(
            serde_json::to_value(Value::Ratio(ratio)).expect("serialize"),
            serde_json::json!({ "unit": "ratio", "value": ratio })
        );
    }
}

#[test]
fn a_non_finite_ratio_is_missing_not_null_under_a_ratio_unit() {
    for ratio in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        // Built through the constructor…
        assert_eq!(Value::ratio(ratio), Value::Missing);
        // …or directly through the variant: the wire shape is the same.
        assert_eq!(
            serde_json::to_value(Value::Ratio(ratio)).expect("serialize"),
            serde_json::json!({ "unit": "missing", "value": null })
        );
    }
}

#[test]
fn a_non_finite_fiat_amount_is_missing_too() {
    assert_eq!(Value::fiat(f64::NAN, "ARS"), Value::Missing);
    assert_eq!(
        Value::fiat(10.0, "ARS"),
        Value::Fiat {
            amount: 10.0,
            code: "ARS".into()
        }
    );
    assert_eq!(
        serde_json::to_value(Value::Fiat {
            amount: f64::INFINITY,
            code: "ARS".into()
        })
        .expect("serialize"),
        serde_json::json!({ "unit": "missing", "value": null })
    );
}

#[test]
fn a_metric_serializes_flat() {
    // One object per metric, with the unit and value beside the name rather
    // than nested under it: `docs/SPEC.md` §10 describes a flat record.
    let json = serde_json::to_value(Metric::observed("orders.created", Value::Count(7)))
        .expect("serialize");

    assert_eq!(
        json,
        serde_json::json!({
            "name": "orders.created",
            "kind": "observed",
            "unit": "count",
            "value": 7,
        })
    );
}

#[test]
fn a_fiat_value_carries_the_currency_it_is_denominated_in() {
    let json = serde_json::to_value(Value::Fiat {
        amount: 1_234.5,
        code: "ARS".into(),
    })
    .expect("serialize");

    assert_eq!(
        json,
        serde_json::json!({
            "unit": "fiat",
            "value": { "amount": 1_234.5, "code": "ARS" },
        })
    );
}
