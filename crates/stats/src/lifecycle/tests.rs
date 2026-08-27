use super::*;

fn version(at: i64, status: Status, fiat: Fiat) -> Version {
    Version {
        at,
        status,
        direction: Direction::Sell,
        fiat_code: "VES".into(),
        amount_sats: 21_000,
        fiat,
        premium: 5.0,
        expires_at: at + 900,
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
fn every_version_is_a_numbered_block_in_the_order_given() {
    // Arrange
    let versions = vec![
        version(100, Status::Pending, Fiat::Fixed(50.0)),
        version(200, Status::InProgress, Fiat::Fixed(50.0)),
        version(300, Status::Success, Fiat::Fixed(50.0)),
    ];
    let fees = vec![FeeSeen {
        at: 400,
        amount_sats: 63,
        is_duplicate: false,
    }];

    // Act
    let metrics = report("abc", &versions, &fees);

    // Assert
    assert_eq!(value(&metrics, "order.id"), &Value::Text("abc".into()));
    assert_eq!(value(&metrics, "order.versions"), &Value::Count(3));
    assert_eq!(
        value(&metrics, "order.1.status"),
        &Value::Text("pending".into())
    );
    assert_eq!(
        value(&metrics, "order.3.status"),
        &Value::Text("success".into())
    );
    assert_eq!(
        value(&metrics, "order.1.at"),
        &Value::Text("1970-01-01T00:01:40+00:00".into())
    );
    assert_eq!(
        value(&metrics, "order.1.fiat"),
        &Value::Fiat {
            amount: 50.0,
            code: "VES".into()
        }
    );
    assert_eq!(value(&metrics, "order.1.premium"), &Value::Ratio(0.05));
    assert_eq!(value(&metrics, "dev_fee.1.amount"), &Value::Sats(63));
    assert_eq!(
        value(&metrics, "dev_fee.1.duplicate"),
        &Value::Text("no".into())
    );
    assert_eq!(metrics.len(), 2 + 3 * 7 + 3);
}

#[test]
fn a_range_order_shows_both_bounds() {
    let versions = vec![version(
        100,
        Status::Pending,
        Fiat::Range {
            min: 10.0,
            max: 100.0,
        },
    )];

    let metrics = report("r", &versions, &[]);

    assert_eq!(
        value(&metrics, "order.1.fiat"),
        &Value::Text("10.00–100.00 VES".into())
    );
}

#[test]
fn an_order_with_no_versions_is_just_its_id() {
    let metrics = report("ghost", &[], &[]);

    assert_eq!(metrics.len(), 2);
    assert_eq!(value(&metrics, "order.versions"), &Value::Count(0));
}

#[test]
fn a_fee_paid_twice_for_one_order_says_which_one_is_the_duplicate() {
    // mostrod has published two fees for one order; the view shows both and
    // names the second, since only the first is money the fund received.
    let versions = vec![version(100, Status::Success, Fiat::Fixed(50.0))];
    let fees = vec![
        FeeSeen {
            at: 400,
            amount_sats: 63,
            is_duplicate: false,
        },
        FeeSeen {
            at: 500,
            amount_sats: 63,
            is_duplicate: true,
        },
    ];

    let metrics = report("abc", &versions, &fees);

    assert_eq!(
        value(&metrics, "dev_fee.1.duplicate"),
        &Value::Text("no".into())
    );
    assert_eq!(
        value(&metrics, "dev_fee.2.duplicate"),
        &Value::Text("yes".into())
    );
}

#[test]
fn an_instant_no_calendar_can_render_is_shown_as_the_number_it_is() {
    // A corrupt `created_at` should not cost the reader the rest of the
    // lifecycle: the field falls back to the timestamp itself.
    // `expires_at` is `at + 900` in the helper, so the instant is picked
    // just below the ceiling: it is the calendar that cannot render it,
    // not the arithmetic that overflows.
    let unrenderable = i64::MAX - 1_000;
    let versions = vec![version(unrenderable, Status::Pending, Fiat::Fixed(50.0))];

    let metrics = report("abc", &versions, &[]);

    assert_eq!(
        value(&metrics, "order.1.at"),
        &Value::Text(unrenderable.to_string())
    );
}
