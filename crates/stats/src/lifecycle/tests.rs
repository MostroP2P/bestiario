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
