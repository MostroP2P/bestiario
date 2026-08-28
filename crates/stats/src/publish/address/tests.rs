//! The `d` grammar of `docs/NOSTR-PUBLICATION.md` §3, as a table.
//!
//! A `d` value is the only thing a client constructs to fetch a document,
//! so every accepted string must round-trip to the same string and every
//! near-miss must be refused — a typo is a miss, never a fuzzy match.

use super::*;

/// Sixty-four hex characters: the parser refused a first draft of this
/// constant that was one character long, which is the guard working.
const PUBKEY: &str = "6320ee5e2ce0e1e0ae5d2a3e0b8f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6fd425";

fn parses(input: &str) -> Address {
    Address::parse(input).unwrap_or_else(|error| panic!("`{input}` should parse: {error}"))
}

fn refused(input: &str) -> ParseError {
    Address::parse(input).expect_err(&format!("`{input}` should be refused"))
}

#[test]
fn every_example_of_the_spec_round_trips() {
    // Arrange: the examples §3 lists, verbatim.
    let examples = [
        "index",
        "summary:30d",
        "orders:7d",
        "volume:30d:n:mainnet",
        "series:orders:daily:2026-01",
        "series:volume:monthly:2026",
        &format!("series:orders:daily:2026-01:i:{PUBKEY}"),
    ];

    for example in examples {
        // Act
        let address = parses(example);

        // Assert
        assert_eq!(address.to_string(), example, "renders back to itself");
    }
}

#[test]
fn the_index_may_be_sharded_by_year() {
    assert_eq!(parses("index"), Address::Index { year: None });
    assert_eq!(parses("index:2026"), Address::Index { year: Some(2026) });
    assert_eq!(
        Address::Index { year: Some(2026) }.to_string(),
        "index:2026",
        "and renders back with the year"
    );
}

#[test]
fn a_window_document_names_a_report_a_window_and_maybe_a_scope() {
    assert_eq!(
        parses("dev-fees:90d"),
        Address::Window {
            report: Report::DevFees,
            window: Window::Days90,
            scope: None,
        }
    );
    assert_eq!(
        parses(&format!("compare:all:i:{PUBKEY}")),
        Address::Window {
            report: Report::Compare,
            window: Window::All,
            scope: Some(Scope::Instance(PUBKEY.to_string())),
        }
    );
}

#[test]
fn a_series_partition_names_its_resolution_and_bucket() {
    assert_eq!(
        parses("series:disputes:weekly:2026-03"),
        Address::Series {
            report: Report::Disputes,
            partition: Partition::new(
                Resolution::Weekly,
                Bucket::Month {
                    year: 2026,
                    month: 3
                }
            )
            .expect("a month of weeks"),
            scope: None,
        }
    );
    assert_eq!(
        parses("series:volume:monthly:2025:n:signet"),
        Address::Series {
            report: Report::Volume,
            partition: Partition::new(Resolution::Monthly, Bucket::Year(2025))
                .expect("a year of months"),
            scope: Some(Scope::Network("signet".to_string())),
        }
    );
}

#[test]
fn a_daily_or_weekly_partition_is_a_month_and_a_monthly_one_is_a_year() {
    // The bucket's shape follows the resolution: a year of days is too big
    // for one document, a year of months is not.
    refused("series:orders:daily:2026");
    refused("series:orders:weekly:2026");
    refused("series:orders:monthly:2026-01");

    // And the same rule holds for an address built rather than parsed, so
    // that `Display` can never emit a string `parse` refuses.
    assert!(Partition::new(Resolution::Daily, Bucket::Year(2026)).is_none());
    assert!(Partition::new(Resolution::Weekly, Bucket::Year(2026)).is_none());
    assert!(
        Partition::new(
            Resolution::Monthly,
            Bucket::Month {
                year: 2026,
                month: 1
            }
        )
        .is_none()
    );
}

#[test]
fn a_typo_is_a_miss_and_never_a_fuzzy_match() {
    for near_miss in [
        "Index",
        "INDEX",
        "index:",
        "index:26",
        "index:2026:",
        "summary",
        "summary:30D",
        "summary:30",
        "summary:30d:",
        "summary:1d",
        "sumary:30d",
        "orders:7d:n:",
        "orders:7d:n:Mainnet",
        "orders:7d:x:mainnet",
        "orders:7d:i:6320ee5e",
        "orders:7d:i:",
        "series:orders:daily:2026-1",
        "series:orders:daily:2026-13",
        "series:orders:daily:2026-00",
        "series:orders:hourly:2026-01",
        "series:orders:daily:2026-01:",
        "series:orders:daily:2026-01:n:mainnet:extra",
        "series::daily:2026-01",
        "series:orders:daily",
        ":summary:30d",
        "summary::30d",
        "",
        " summary:30d",
        "summary:30d ",
    ] {
        refused(near_miss);
    }
}

#[test]
fn an_instance_scope_is_the_full_pubkey_in_lowercase_hex() {
    // A prefix is a collision waiting to be found; uppercase is a
    // different string to a relay's `#d` filter.
    refused(&format!("orders:7d:i:{}", &PUBKEY[..32]));
    refused(&format!("orders:7d:i:{}", PUBKEY.to_uppercase()));
    refused(&format!("orders:7d:i:{}g", &PUBKEY[..63]));
    assert!(Address::parse(&format!("orders:7d:i:{PUBKEY}")).is_ok());
}

#[test]
fn a_network_scope_is_one_the_indexer_knows() {
    for network in ["mainnet", "testnet", "signet", "regtest"] {
        assert!(
            Address::parse(&format!("orders:7d:n:{network}")).is_ok(),
            "{network}"
        );
    }
    refused("orders:7d:n:bitcoin");
}

#[test]
fn the_refusal_says_which_part_was_wrong() {
    let error = refused("summary:30D");
    assert!(error.to_string().contains("window"), "{error}");
    assert!(error.to_string().contains("30D"), "{error}");

    let error = refused("sumary:30d");
    assert!(error.to_string().contains("report"), "{error}");

    let error = refused("series:orders:daily:2026-13");
    assert!(error.to_string().contains("bucket"), "{error}");
}

#[test]
fn every_report_window_and_resolution_renders_as_the_grammar_spells_it() {
    assert_eq!(Report::DevFees.as_str(), "dev-fees");
    assert_eq!(Window::Hours24.as_str(), "24h");
    assert_eq!(Resolution::Daily.as_str(), "daily");
    for report in Report::ALL {
        assert_eq!(Report::parse(report.as_str()), Some(report));
    }
    for window in Window::ALL {
        assert_eq!(Window::parse(window.as_str()), Some(window));
    }
    for resolution in Resolution::ALL {
        assert_eq!(Resolution::parse(resolution.as_str()), Some(resolution));
    }
}

#[test]
fn a_weekly_partition_is_filed_under_the_month_its_first_day_falls_in() {
    // 2025-12-29 is a Monday; that week runs into January and lives in
    // December's partition only.
    let week_start = chrono::NaiveDate::from_ymd_opt(2025, 12, 29).expect("a date");

    assert_eq!(
        Bucket::for_week_starting(week_start),
        Bucket::Month {
            year: 2025,
            month: 12
        }
    );
}

#[test]
fn a_partition_spans_its_whole_month_or_year_and_december_rolls_the_year() {
    // 2026-08-01 to 2026-09-01; 2026-12-01 to 2027-01-01; 2026 whole.
    let august = Partition::new(
        Resolution::Daily,
        Bucket::Month {
            year: 2026,
            month: 8,
        },
    )
    .expect("a month")
    .window();
    assert_eq!((august.from, august.until), (1_785_542_400, 1_788_220_800));

    let december = Bucket::Month {
        year: 2026,
        month: 12,
    }
    .window();
    assert_eq!(
        (december.from, december.until),
        (1_796_083_200, 1_798_761_600)
    );

    let year = Partition::new(Resolution::Monthly, Bucket::Year(2026))
        .expect("a year")
        .window();
    assert_eq!((year.from, year.until), (1_767_225_600, 1_798_761_600));
    assert_eq!(
        year.until, december.until,
        "the year ends where December does"
    );
}
