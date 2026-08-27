use super::*;

fn snapshot(pubkey: &str, published_at: i64, usd: f64) -> Snapshot {
    Snapshot {
        pubkey: pubkey.to_string(),
        published_at,
        rates: BTreeMap::from([("USD".to_string(), usd), ("ARS".to_string(), usd * 1_000.0)]),
    }
}

/// alpha at 1000, 2000 and 3000; beta at 1500 and 2050.
fn book() -> RateBook {
    RateBook::new(vec![
        snapshot("alpha", 1_000, 50_000.0),
        snapshot("beta", 1_500, 51_000.0),
        snapshot("alpha", 2_000, 52_000.0),
        snapshot("beta", 2_050, 51_500.0),
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
    let quote = book().rate_at("alpha", "USD", 2_200).expect("a rate");

    assert_eq!(
        quote.rate, 52_000.0,
        "the newest at or before, not the newest"
    );
    assert_eq!(quote.age_secs, 200);
}

#[test]
fn a_snapshot_from_after_the_instant_is_never_used() {
    // At 1100 alpha's 2000 snapshot is in the future; its 1000 one is not.
    let quote = book().rate_at("alpha", "USD", 1_100).expect("a rate");

    assert_eq!(quote.rate, 50_000.0);
    assert_eq!(quote.age_secs, 100);
}

#[test]
fn a_snapshot_older_than_the_bound_is_not_a_rate() {
    // Five minutes exactly still qualifies; a second more does not.
    let at_the_bound = book().rate_at("alpha", "USD", 1_000 + MAX_AGE_SECS);
    assert_eq!(at_the_bound.expect("a rate").age_secs, MAX_AGE_SECS);

    let past_it = book().rate_at("alpha", "USD", 1_000 + MAX_AGE_SECS + 1);
    assert_eq!(past_it, None, "nobody else had one either at 1301");
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
    // At 2100 alpha's own is from 2000; beta has a fresher one from 2050.
    // The instance's own rate is what it settled at.
    let quote = book().rate_at("alpha", "USD", 2_100).expect("a rate");

    assert_eq!(quote.rate, 52_000.0);
    assert_eq!(quote.source, RateSource::Instance);
}

#[test]
fn an_instance_s_own_stale_snapshot_yields_to_another_s_usable_one() {
    // At 1700 alpha's own is 700 old — not a rate any more — while beta's
    // 1500 one still is. The fallback says so.
    let quote = book().rate_at("alpha", "USD", 1_700).expect("a rate");

    assert_eq!(quote.rate, 51_000.0);
    assert_eq!(quote.age_secs, 200);
    assert_eq!(
        quote.source,
        RateSource::Fallback {
            pubkey: "beta".into()
        }
    );
}

#[test]
fn a_currency_nobody_quoted_has_no_rate() {
    assert_eq!(book().rate_at("alpha", "XYZ", 2_000), None);
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
            published_at: 1_100,
            rates: BTreeMap::from([("EUR".to_string(), 45_000.0)]),
        },
    ]);

    let quote = book.rate_at("alpha", "USD", 1_200).expect("a rate");

    assert_eq!(quote.rate, 50_000.0);
    assert_eq!(quote.age_secs, 200);
}

#[test]
fn the_lookup_lands_on_the_right_snapshot_in_a_long_history() {
    // A year of hourly snapshots from two instances: the answer at any
    // instant is the one just before it, whoever the history is scanned.
    let hourly: Vec<Snapshot> = (0..(2 * 365 * 24))
        .map(|i| {
            snapshot(
                if i % 2 == 0 { "alpha" } else { "beta" },
                i * 1_800,
                i as f64,
            )
        })
        .collect();
    let book = RateBook::new(hourly);

    let quote = book
        .rate_at("alpha", "USD", 10_000 * 1_800 + 60)
        .expect("a rate");
    assert_eq!(quote.rate, 10_000.0);
    assert_eq!(quote.age_secs, 60);

    let fallback = book
        .rate_at("beta", "USD", 10_000 * 1_800 + 60)
        .expect("a rate");
    assert_eq!(
        fallback.rate, 10_000.0,
        "beta's own is 1860 old; alpha's is usable"
    );
    assert!(matches!(fallback.source, RateSource::Fallback { .. }));
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

#[test]
fn a_fallback_lookup_does_not_walk_more_history_than_the_bound_allows() {
    // The fallback used to build a fresh index of every snapshot on each
    // call, so an unquoted currency cost the whole archive per order. This
    // pins the answer; the cost is what the index in `RateBook` is for.
    let mut snapshots = Vec::new();
    for tick in 0..5_000i64 {
        snapshots.push(Snapshot {
            pubkey: format!("other-{}", tick % 7),
            published_at: tick * 10,
            rates: BTreeMap::from([("USD".to_string(), 50_000.0 + tick as f64)]),
        });
    }
    let book = RateBook::new(snapshots);

    let quote = book.rate_at("silent", "USD", 49_990).expect("a fallback");

    assert_eq!(quote.rate, 50_000.0 + 4_999.0);
    assert!(matches!(quote.source, RateSource::Fallback { .. }));
    assert!(book.rate_at("silent", "EUR", 49_990).is_none());
}

#[test]
fn two_instances_publishing_in_the_same_second_are_ordered_by_pubkey() {
    // The book is walked backwards to the instant asked about, so two
    // snapshots sharing a second need a total order or the walk is not
    // reproducible from one run to the next.
    let book = RateBook::new(vec![
        snapshot("beta", 1_000, 51_000.0),
        snapshot("alpha", 1_000, 50_000.0),
    ]);

    // Both stand at the same instant; the lookup finds each instance's own.
    assert_eq!(
        book.rate_at("alpha", "USD", 1_000).expect("a rate").rate,
        50_000.0
    );
    assert_eq!(
        book.rate_at("beta", "USD", 1_000).expect("a rate").rate,
        51_000.0
    );

    // An instance with no snapshot of its own is answered from the shared
    // index instead, where the same tie is broken the same way: the later
    // pubkey sorts last and is what a backwards walk reaches first.
    let fallback = book.rate_at("gamma", "USD", 1_000).expect("a rate");
    assert_eq!(fallback.rate, 51_000.0);
    assert_eq!(
        fallback.source,
        RateSource::Fallback {
            pubkey: "beta".into()
        }
    );
}
