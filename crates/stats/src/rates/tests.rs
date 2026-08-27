use super::*;

fn snapshot(pubkey: &str, published_at: i64, usd: f64) -> Snapshot {
    Snapshot {
        pubkey: pubkey.to_string(),
        published_at,
        rates: BTreeMap::from([("USD".to_string(), usd), ("ARS".to_string(), usd * 1_000.0)]),
    }
}

fn book() -> RateBook {
    RateBook::new(vec![
        snapshot("alpha", 1_000, 50_000.0),
        snapshot("beta", 1_500, 51_000.0),
        snapshot("alpha", 2_000, 52_000.0),
        snapshot("alpha", 3_000, 53_000.0),
    ])
}

#[test]
fn an_exact_hit_has_no_age() {
    // Arrange / Act
    let quote = book().rate_at("alpha", "USD", 2_000).expect("a rate");

    // Assert
    assert_eq!(quote.rate, 52_000.0);
    assert_eq!(quote.age_secs, 0);
    assert_eq!(quote.source, RateSource::Instance);
}

#[test]
fn a_stale_hit_reports_how_stale() {
    let quote = book().rate_at("alpha", "USD", 2_600).expect("a rate");

    assert_eq!(
        quote.rate, 52_000.0,
        "the newest at or before, not the newest"
    );
    assert_eq!(quote.age_secs, 600);
}

#[test]
fn a_snapshot_from_after_the_instant_is_never_used() {
    let quote = book().rate_at("alpha", "USD", 1_999).expect("a rate");

    assert_eq!(quote.rate, 50_000.0);
    assert_eq!(quote.age_secs, 999);
}

#[test]
fn the_fallback_names_the_instance_it_came_from() {
    // Gamma never published; beta's 1500 snapshot is the newest at 1800.
    let quote = book().rate_at("gamma", "USD", 1_800).expect("a rate");

    assert_eq!(quote.rate, 51_000.0);
    assert_eq!(quote.age_secs, 300);
    assert_eq!(
        quote.source,
        RateSource::Fallback {
            pubkey: "beta".into()
        }
    );
}

#[test]
fn the_instance_s_own_older_snapshot_beats_another_s_newer_one() {
    // At 1800 alpha's own is from 1000; beta has a fresher one from 1500.
    // The instance's own rate is what it settled at.
    let quote = book().rate_at("alpha", "USD", 1_800).expect("a rate");

    assert_eq!(quote.rate, 50_000.0);
    assert_eq!(quote.source, RateSource::Instance);
}

#[test]
fn a_currency_nobody_quoted_has_no_rate() {
    assert_eq!(book().rate_at("alpha", "XYZ", 5_000), None);
}

#[test]
fn nothing_before_the_instant_has_no_rate() {
    assert_eq!(book().rate_at("alpha", "USD", 999), None);
    assert!(RateBook::default().rate_at("alpha", "USD", 5_000).is_none());
}

#[test]
fn a_snapshot_without_the_currency_is_skipped_for_an_older_one_that_has_it() {
    let book = RateBook::new(vec![
        snapshot("alpha", 1_000, 50_000.0),
        Snapshot {
            pubkey: "alpha".into(),
            published_at: 2_000,
            rates: BTreeMap::from([("EUR".to_string(), 45_000.0)]),
        },
    ]);

    let quote = book.rate_at("alpha", "USD", 2_500).expect("a rate");

    assert_eq!(quote.rate, 50_000.0);
    assert_eq!(quote.age_secs, 1_500);
}

#[test]
fn sats_convert_at_the_rate() {
    let quote = RateQuote {
        rate: 50_000.0,
        age_secs: 0,
        source: RateSource::Instance,
    };

    assert_eq!(quote.convert_sats(100_000_000), 50_000.0);
    assert_eq!(quote.convert_sats(1_000_000), 500.0);
}
