//! A snapshot's payloads, hashed — `docs/NOSTR-PUBLICATION.md` §5–§6 as a
//! table over a hand-built archive.

use super::*;
use crate::activity::{Direction, Order, Origin, Status};
use crate::bucket::Coverage;
use crate::instances::Profile;
use crate::metric::{Metric, Value};
use crate::publish::address::{Bucket, Month, Report, Resolution, Window as Span, Year};

fn month_of(year: i32, month: u32) -> Bucket {
    Bucket::Month {
        year: Year::new(year).expect("a four-digit year"),
        month: Month::new(month).expect("a month of the year"),
    }
}

fn year_of(year: i32) -> Bucket {
    Bucket::Year(Year::new(year).expect("a four-digit year"))
}
use crate::series::Data;
use crate::window::Window;

/// 2026-07-01, 2026-08-01 and 2026-09-01 at midnight UTC.
const JULY: i64 = 1_782_864_000;
const AUGUST: i64 = 1_785_542_400;
const SEPTEMBER: i64 = 1_788_220_800;
const DAY: i64 = 86_400;

fn order(id: &str, created_at: i64, sats: i64) -> Order {
    Order {
        order_id: id.to_string(),
        pubkey: "pk".into(),
        instance: "Alpha (pk)".into(),
        created_at,
        status: Status::Success,
        direction: Direction::Buy,
        fiat_code: "ARS".into(),
        payment_methods: vec!["cash".into()],
        amount_sats: sats,
        fiat_amount: Some(50.0),
        premium: 0.0,
        is_market_price: false,
        fiat_range: None,
        pending_at: Some(created_at),
        origin: Origin {
            fiat_code: "ARS".into(),
            payment_methods: vec!["cash".into()],
            direction: Direction::Buy,
        },
        taken_at: Some(created_at + 60),
        success_at: Some(created_at + DAY),
        canceled_at: None,
        expires_at: Some(created_at + 2 * DAY),
    }
}

fn profile(pubkey: &str, name: &str) -> Profile {
    Profile {
        pubkey: pubkey.to_string(),
        name: Some(name.to_string()),
        label: format!("{name} ({pubkey})"),
        mostro_version: Some("0.14.2".into()),
        protocol_version: Some("1".into()),
        fee: Some(0.006),
        min_order_sats: Some(1_000),
        max_order_sats: Some(1_000_000),
        fiat_currencies: vec!["ARS".into()],
        ln_networks: vec!["mainnet".into()],
        bond_enabled: Some(true),
        first_seen_at: JULY,
        last_seen_at: AUGUST + DAY,
    }
}

/// One order completed in July, two in August.
fn data() -> Data {
    Data {
        orders: vec![
            order("j1", JULY + DAY, 5_000),
            order("a1", AUGUST + DAY, 10_000),
            order("a2", AUGUST + 3 * DAY, 30_000),
        ],
        ..Data::default()
    }
}

fn august() -> Bucket {
    month_of(2026, 8)
}

fn payload_of(partition: &Partition) -> serde_json::Value {
    serde_json::to_value(&partition.payload).expect("serialises")
}

fn column_names(json: &serde_json::Value) -> Vec<String> {
    json["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|column| column["name"].as_str().expect("name").to_string())
        .collect()
}

// ---- window documents (§6.1)

#[test]
fn a_window_payload_is_the_report_envelope_without_its_clock() {
    // Arrange
    let metrics = vec![
        Metric::observed("orders.created", Value::Count(3)),
        Metric::inferred("volume.in.USD.total", Value::Missing, "no rate used"),
    ];

    // Act
    let payload = window_payload(Window::new(JULY, SEPTEMBER), &metrics);
    let json = serde_json::to_value(&payload).expect("serialises");

    // Assert: `range` and `metrics` verbatim, and nothing about the run.
    assert_eq!(json["range"]["from"], "2026-07-01T00:00:00+00:00");
    assert_eq!(json["range"]["until"], "2026-09-01T00:00:00+00:00");
    assert_eq!(json["metrics"][0]["name"], "orders.created");
    assert_eq!(json["metrics"][0]["kind"], "observed");
    assert_eq!(json["metrics"][0]["unit"], "count");
    assert_eq!(json["metrics"][0]["value"], 3);
    assert_eq!(json["metrics"][1]["kind"], "inferred");
    assert_eq!(json["metrics"][1]["error"], "no rate used");
    assert!(json.get("generated_at").is_none(), "belongs to the run");
    assert!(json.get("snapshot_id").is_none());
}

// ---- series partitions (§6.2)

fn august_daily() -> Partition {
    partition(
        &data(),
        Report::Orders,
        Resolution::Daily,
        august(),
        Coverage::since(JULY),
        SEPTEMBER,
    )
    .expect("inside coverage")
}

#[test]
fn a_partition_is_columnar_with_the_bucket_as_its_first_column() {
    let json = payload_of(&august_daily());

    assert_eq!(json["resolution"], "daily");
    assert_eq!(json["period"]["from"], "2026-08-01T00:00:00+00:00");
    assert_eq!(json["period"]["until"], "2026-09-01T00:00:00+00:00");
    assert_eq!(json["columns"][0]["name"], "date");
    assert_eq!(json["columns"][0]["unit"], "date");
    assert_eq!(
        json["rows"].as_array().expect("rows").len(),
        31,
        "one per day, none skipped"
    );
    assert_eq!(json["rows"][0][0], "2026-08-01");
    assert_eq!(json["rows"][30][0], "2026-08-31");
}

#[test]
fn every_metric_of_the_family_is_a_column_declared_once() {
    let json = payload_of(&august_daily());
    let columns = column_names(&json);

    assert!(columns.contains(&"created".to_string()), "{columns:?}");
    assert!(columns.contains(&"completed".to_string()), "{columns:?}");
    assert!(
        columns.contains(&"completion_rate".to_string()),
        "{columns:?}"
    );
    // `kind` and `unit` once per column, not once per cell.
    let created = json["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|column| column["name"] == "created")
        .expect("created");
    assert_eq!(created["kind"], "observed");
    assert_eq!(created["unit"], "count");
    // A figure about now, and a delta, have no place in a partition.
    assert!(
        !columns.iter().any(|name| name.ends_with("_now")),
        "{columns:?}"
    );
    assert!(
        !columns.iter().any(|name| name.ends_with("_delta")),
        "{columns:?}"
    );
}

#[test]
fn a_row_holds_the_figures_of_its_bucket_in_column_order() {
    let json = payload_of(&august_daily());
    let columns = column_names(&json);
    let created = columns.iter().position(|name| name == "created").unwrap();
    let rate = columns
        .iter()
        .position(|name| name == "completion_rate")
        .unwrap();

    let completed = columns.iter().position(|name| name == "completed").unwrap();

    // 2026-08-02: `a1` is created. 2026-08-03: nothing is created, but
    // `a1` reaches `success` — completions are dated by `success_at`, the
    // rule of every report — so the day has a completion and a rate.
    assert_eq!(json["rows"][1][created], 1);
    assert_eq!(json["rows"][2][created], 0);
    assert_eq!(json["rows"][2][completed], 1);
    assert_eq!(json["rows"][2][rate], 1.0);
    // 2026-08-06: nothing at all. Counts are real zeros; a rate over
    // nothing is absent, not zero — the `—` of the tables.
    assert_eq!(json["rows"][5][created], 0, "a quiet day is a real zero");
    assert_eq!(json["rows"][5][completed], 0);
    assert!(
        json["rows"][5][rate].is_null(),
        "row 5 = {:?}",
        json["rows"][5]
    );
}

#[test]
fn an_inferred_column_carries_its_error_once() {
    let priced = Data {
        priced: Some(crate::series::Priced {
            book: crate::rates::RateBook::default(),
            code: "USD".into(),
        }),
        ..data()
    };

    let partition = partition(
        &priced,
        Report::Volume,
        Resolution::Monthly,
        year_of(2026),
        Coverage::since(JULY),
        SEPTEMBER,
    )
    .expect("inside coverage");
    let json = payload_of(&partition);
    let column = json["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|column| column["name"] == "in.USD.total")
        .expect("the converted total");

    assert_eq!(column["kind"], "inferred");
    assert!(column["error"].as_str().is_some(), "{column}");
}

// ---- the cells, and the unit a column declares

#[test]
fn every_kind_of_figure_has_a_cell_and_a_unit() {
    // The table exists because a column declares its unit once and every
    // cell under it is then a bare figure: the two have to agree for all
    // seven variants, including the ones no family plots today, or the
    // first metric that returns one publishes a number under the wrong
    // word.
    let cases = [
        (Value::Count(7), "count", serde_json::json!(7)),
        (Value::Sats(9_000), "sats", serde_json::json!(9_000)),
        (Value::ratio(0.25), "ratio", serde_json::json!(0.25)),
        (Value::Seconds(90), "seconds", serde_json::json!(90)),
        (Value::fiat(12.5, "ARS"), "fiat", serde_json::json!(12.5)),
        (
            Value::Text("cash".into()),
            "text",
            serde_json::json!("cash"),
        ),
        (Value::Missing, "missing", serde_json::Value::Null),
    ];

    for (value, unit, expected) in cases {
        assert_eq!(unit_of(&value), unit, "unit of {value:?}");
        assert_eq!(cell(&value), expected, "cell of {value:?}");
    }
}

#[test]
fn a_figure_that_is_not_a_number_is_absent_rather_than_published() {
    // `normalised` is what decides this, and both the column and the cell
    // go through it: a `NaN` must not reach a reader as a unit of "ratio"
    // over a `null`, which would read as a rate the run failed to compute
    // rather than as a rate that does not exist.
    let nonsense = Value::Ratio(f64::NAN);

    assert_eq!(unit_of(&nonsense), "missing");
    assert_eq!(cell(&nonsense), serde_json::Value::Null);
}

// ---- the currency blocks of two documents (§7)

#[test]
fn the_currency_blocks_of_orders_and_volume_count_the_same_orders() {
    // NOSTR-CLIENT §7 tells a client the two documents agree, and that a
    // range order — in none of the fiat figures, since it names no single
    // amount — is in the sats and the count all the same.
    let mut data = data();
    data.orders.push(Order {
        fiat_code: "BRL".into(),
        fiat_amount: None,
        ..order("range", AUGUST + 2 * DAY, 7_000)
    });
    let window = Window::new(JULY, SEPTEMBER);

    let orders = window_metrics(Report::Orders, &data, window, SEPTEMBER);
    let volume = window_metrics(Report::Volume, &data, window, SEPTEMBER);
    let value = |metrics: &[Metric], name: &str| {
        metrics
            .iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("{name} is published"))
            .value
            .clone()
    };

    for code in ["ARS", "BRL"] {
        assert_eq!(
            value(&volume, &format!("volume.fiat.{code}.completed")),
            value(&orders, &format!("orders.{code}.completed")),
            "{code}"
        );
    }
    assert_eq!(value(&volume, "volume.fiat.BRL.sats"), Value::Sats(7_000));
    assert_eq!(
        value(&volume, "volume.fiat.BRL.total"),
        Value::Missing,
        "a range order names no fiat amount to total"
    );

    // And the currencies partition the window's sats.
    let sats: i64 = volume
        .iter()
        .filter(|metric| metric.name.starts_with("volume.fiat.") && metric.name.ends_with(".sats"))
        .map(|metric| match metric.value {
            Value::Sats(sats) => sats,
            ref other => panic!("{other:?} is not sats"),
        })
        .sum();
    assert_eq!(value(&volume, "volume.sats"), Value::Sats(sats));
}

// ---- absence (§6.3)

#[test]
fn a_bucket_outside_coverage_is_null_in_every_column_counts_included() {
    // The archive begins on the 15th: the first fortnight was never seen.
    let partition = partition(
        &data(),
        Report::Orders,
        Resolution::Daily,
        august(),
        Coverage::since(AUGUST + 14 * DAY),
        SEPTEMBER,
    )
    .expect("partly inside coverage");
    let json = payload_of(&partition);
    let row = json["rows"][0].as_array().expect("a row");

    assert_eq!(row[0], "2026-08-01", "the row is there");
    assert!(
        row[1..].iter().all(serde_json::Value::is_null),
        "and says nothing: {row:?}"
    );
    // While a day inside coverage with no orders is a real zero.
    let quiet = json["rows"][20].as_array().unwrap();
    assert_eq!(quiet[1], 0);
}

#[test]
fn a_partition_entirely_outside_coverage_is_not_a_document() {
    let before_the_archive = partition(
        &data(),
        Report::Orders,
        Resolution::Daily,
        month_of(2026, 6),
        Coverage::since(JULY),
        SEPTEMBER,
    );

    assert!(before_the_archive.is_none());
}

// ---- the hash (§5)

#[test]
fn the_hash_is_over_the_payload_and_the_same_figures_give_the_same_bytes() {
    let once = august_daily();
    let again = august_daily();

    assert_eq!(once.hash, again.hash);
    assert_eq!(once.hash.len(), 64, "sha-256, hex");
    assert!(
        once.hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(once.hash, hash_of(&once.payload));
}

#[test]
fn a_changed_figure_changes_the_hash_and_a_changed_clock_does_not() {
    let base = august_daily();

    let mut orders = data().orders;
    orders.push(order("a3", AUGUST + 5 * DAY, 1_000));
    let changed = partition(
        &Data {
            orders,
            ..Data::default()
        },
        Report::Orders,
        Resolution::Daily,
        august(),
        Coverage::since(JULY),
        SEPTEMBER,
    )
    .unwrap();
    assert_ne!(base.hash, changed.hash);

    // Computed a week later over the same archive: the figures did not move.
    let later = partition(
        &data(),
        Report::Orders,
        Resolution::Daily,
        august(),
        Coverage::since(JULY),
        SEPTEMBER + 7 * DAY,
    )
    .unwrap();
    assert_eq!(base.hash, later.hash);
}

// ---- the whole snapshot (§7)

#[test]
fn every_document_of_a_snapshot_shares_one_id_and_one_clock() {
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);

    assert!(snapshot.documents.len() > 1);
    for document in &snapshot.documents {
        assert_eq!(document.envelope.snapshot_id(), "01J8ZRUN");
        assert_eq!(
            document.envelope.generated_at(),
            "2026-09-01T00:00:00+00:00"
        );
    }
}

#[test]
fn a_snapshot_publishes_every_window_report_and_every_covered_partition() {
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let addresses: Vec<String> = snapshot
        .documents
        .iter()
        .map(|document| document.address.to_string())
        .collect();
    let has = |address: &str| addresses.iter().any(|found| found == address);

    assert!(has("orders:30d"), "{addresses:?}");
    assert!(has("volume:all"), "{addresses:?}");
    assert!(has("series:orders:daily:2026-07"), "{addresses:?}");
    assert!(has("series:orders:daily:2026-08"), "{addresses:?}");
    assert!(has("series:orders:monthly:2026"), "{addresses:?}");
    assert!(
        !has("series:orders:daily:2026-06"),
        "nothing before the archive: {addresses:?}"
    );
    // No index yet: that is the next row, and it is built from these.
    assert!(!has("index"));
}

#[test]
fn a_run_over_an_archive_that_has_not_moved_gives_every_window_the_same_hash() {
    // Arrange: one archive, read by two runs a minute apart. §8: "a run
    // over an archive that has not moved re-sends no document" — which
    // holds only if the window a document covers is a function of the
    // archive rather than of the clock that happened to read it.
    let archive = Coverage::between(JULY, AUGUST);

    // Act
    let once = Snapshot::compute(&data(), archive, "a", SEPTEMBER);
    let again = Snapshot::compute(&data(), archive, "b", SEPTEMBER + 60);

    // Assert: every window document, not just the closed partitions.
    let windows = |snapshot: &Snapshot| -> Vec<(String, String)> {
        snapshot
            .documents
            .iter()
            .filter(|document| document.period.is_none())
            .map(|document| (document.address.to_string(), document.hash.clone()))
            .collect()
    };
    let before = windows(&once);
    assert!(!before.is_empty(), "the snapshot has window documents");
    assert_eq!(
        before,
        windows(&again),
        "the clock moved and the archive did not, so no window figure moved"
    );
}

#[test]
fn a_window_ends_at_the_archives_ceiling_and_not_at_the_clock() {
    // Arrange: an ingest an hour behind the clock. A window running to
    // `now` would count that hour as a period with no activity, which is
    // the flat line at zero §6.3 exists to refuse.
    let archive = Coverage::between(JULY, AUGUST);

    // Act
    let snapshot = Snapshot::compute(&data(), archive, "a", AUGUST + 3600);
    let payload = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "orders:24h")
        .map(|document| document.envelope.payload().clone())
        .expect("a 24h window document");

    // Assert: it ends where the archive does.
    assert_eq!(payload["range"]["until"], rfc3339(AUGUST));
    assert_eq!(payload["range"]["from"], rfc3339(AUGUST - 86_400));
}

#[test]
fn a_ceiling_beyond_the_clock_does_not_reach_past_it() {
    // A relay can serve an event dated in the future, and a window is
    // not the place to publish one.
    let snapshot = Snapshot::compute(
        &data(),
        Coverage::between(JULY, SEPTEMBER + 86_400),
        "a",
        SEPTEMBER,
    );
    let payload = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "orders:24h")
        .map(|document| document.envelope.payload().clone())
        .expect("a 24h window document");

    assert_eq!(payload["range"]["until"], rfc3339(SEPTEMBER));
}

#[test]
fn an_archive_dated_entirely_in_the_future_does_not_open_a_window_after_it_closes() {
    // Arrange: a relay served events dated ahead of the clock, so the
    // archive's floor and ceiling are both past `now`. The ceiling is
    // clamped to the clock; the floor `all` reaches back to must be
    // clamped with it, or the window opens after it closes.
    let snapshot = Snapshot::compute(
        &data(),
        Coverage::between(SEPTEMBER + 86_400, SEPTEMBER + 2 * 86_400),
        "a",
        SEPTEMBER,
    );

    // Act
    let payload = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "orders:all")
        .map(|document| document.envelope.payload().clone())
        .expect("an `all` window document");

    // Assert: empty, and not inverted.
    assert_eq!(payload["range"]["from"], rfc3339(SEPTEMBER));
    assert_eq!(payload["range"]["until"], rfc3339(SEPTEMBER));
}

#[test]
fn the_documents_of_a_snapshot_are_in_a_stable_order() {
    let once = Snapshot::compute(&data(), Coverage::since(JULY), "a", SEPTEMBER);
    let again = Snapshot::compute(&data(), Coverage::since(JULY), "b", SEPTEMBER);

    let order_once: Vec<String> = once
        .documents
        .iter()
        .map(|d| d.address.to_string())
        .collect();
    let order_again: Vec<String> = again
        .documents
        .iter()
        .map(|d| d.address.to_string())
        .collect();
    assert_eq!(order_once, order_again);
}

// ---- a week belongs to one month, and a bucket that has not happened
// yet is absent (§3, §6.3)

fn july() -> Bucket {
    month_of(2026, 7)
}

fn weekly(bucket: Bucket) -> Partition {
    partition(
        &data(),
        Report::Orders,
        Resolution::Weekly,
        bucket,
        Coverage::since(JULY),
        SEPTEMBER,
    )
    .expect("inside coverage")
}

fn keys(partition: &Partition) -> Vec<String> {
    partition
        .payload
        .rows
        .iter()
        .map(|row| row[0].as_str().expect("key").to_string())
        .collect()
}

#[test]
fn a_week_straddling_a_month_is_filed_under_one_partition_only() {
    let july = keys(&weekly(july()));
    let august = keys(&weekly(august()));

    assert!(
        !july.iter().any(|week| august.contains(week)),
        "one ISO week, one key, one set of figures: july={july:?} august={august:?}"
    );
    // 2026-08-01 is a Saturday, so the week it falls in opened on
    // 2026-07-27 and is July's — the month its first day falls in.
    assert_eq!(july.last().map(String::as_str), Some("2026-W31"));
    assert_eq!(august.first().map(String::as_str), Some("2026-W32"));

    // And every row opens on a Monday, a week after the one before it —
    // the property an off-by-one day in the week arithmetic breaks while
    // still producing the right number of rows.
    let mondays = rows_of(
        Resolution::Weekly,
        super::super::address::Partition::new(Resolution::Weekly, month_of(2026, 8))
            .expect("a weekly month")
            .window(),
    );
    assert_eq!(mondays.len(), 5);
    for (index, (key, week)) in mondays.iter().enumerate() {
        assert_eq!(*key, format!("2026-W{}", 32 + index));
        assert_eq!(week.until - week.from, 7 * 86_400);
        assert_eq!(
            (week.from + 3 * 86_400).rem_euclid(7 * 86_400),
            0,
            "{key} does not open on a Monday"
        );
    }
}

#[test]
fn a_weekly_row_is_a_whole_week_rather_than_the_part_inside_the_month() {
    let august = weekly(august());
    let json = payload_of(&august);
    let columns = column_names(&json);
    let created = columns.iter().position(|name| name == "created").unwrap();

    // The August partition opens on the 3rd, not the 1st: the days before
    // it were counted in July's last week, and counting them twice is the
    // failure a clipped week produces.
    assert_eq!(json["period"]["from"], "2026-08-01T00:00:00+00:00");
    assert_eq!(keys(&august).len(), 5, "the five weeks that open in August");
    assert!(
        json["rows"][0][created].is_number(),
        "a whole week is still a week the archive can speak for"
    );
}

#[test]
fn a_bucket_that_has_not_happened_yet_is_absent_rather_than_zero() {
    // now = 2026-08-04, three days into the month the partition covers.
    let now = AUGUST + 3 * DAY;
    let partition = partition(
        &data(),
        Report::Orders,
        Resolution::Daily,
        august(),
        Coverage::since(JULY),
        now,
    )
    .expect("inside coverage");
    let json = payload_of(&partition);
    let columns = column_names(&json);
    let created = columns.iter().position(|name| name == "created").unwrap();

    assert_eq!(json["rows"].as_array().expect("rows").len(), 31);
    assert_eq!(
        json["rows"][2][created], 0,
        "a quiet day that happened is a real zero"
    );
    for day in 3..31 {
        assert!(
            json["rows"][day][created].is_null(),
            "row {day} = {:?}: a chart that dips to zero for the rest of the month \
             is a more convincing lie than one that stops",
            json["rows"][day]
        );
    }
}

// ---- every report gets its window documents (§3, §6.1)

#[test]
fn a_snapshot_publishes_a_window_document_for_every_report_and_window() {
    // Arrange
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let addresses: Vec<String> = snapshot
        .documents
        .iter()
        .map(|document| document.address.to_string())
        .collect();

    // Act / Assert — the grammar of §3 names eight reports, and a client
    // that constructs one of those addresses must not get a miss.
    for report in Report::ALL {
        for window in Span::ALL {
            let address = format!("{}:{}", report.as_str(), window.as_str());
            assert!(
                addresses.contains(&address),
                "{address} is missing from {addresses:?}"
            );
        }
    }
}

#[test]
fn a_report_with_no_series_family_still_gets_its_windows() {
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let has = |address: &str| {
        snapshot
            .documents
            .iter()
            .any(|document| document.address.to_string() == address)
    };

    // These four have no shape over time, which is a fact about their
    // series and not about their windows.
    assert!(has("summary:30d"));
    assert!(has("market:30d"));
    assert!(has("instances:30d"));
    assert!(has("compare:30d"));
}

#[test]
fn a_report_with_no_series_family_gets_no_partitions() {
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);

    for document in &snapshot.documents {
        let address = document.address.to_string();
        for report in ["summary", "market", "instances", "compare"] {
            assert!(
                !address.starts_with(&format!("series:{report}:")),
                "{address} plots a report that has no shape over time"
            );
        }
    }
}

#[test]
fn the_instances_document_names_the_instances_that_traded() {
    // Arrange — the archive knows one instance, and it created orders.
    let data = Data {
        profiles: vec![profile("pk", "Alpha")],
        ..data()
    };

    // Act
    let snapshot = Snapshot::compute(&data, Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let document = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "instances:all")
        .expect("an instances window document");
    let payload = serde_json::to_value(document.envelope.payload()).expect("serialises");
    let names: Vec<String> = payload["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .map(|metric| metric["name"].as_str().expect("name").to_string())
        .collect();

    // Assert — this is what the map on the site needs and cannot get
    // anywhere else: which instance, under which name, trading how much.
    assert!(
        names
            .iter()
            .any(|name| name.contains("Alpha") && name.ends_with(".created")),
        "{names:?}"
    );
}

#[test]
fn an_archive_with_no_instances_publishes_an_empty_instances_document() {
    // An archive that knows no profiles has nothing to say about them, and
    // says nothing — rather than not publishing the document, which a
    // client cannot tell from a relay withholding it.
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let document = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "instances:all")
        .expect("an instances window document");
    let payload = serde_json::to_value(document.envelope.payload()).expect("serialises");

    assert_eq!(payload["metrics"].as_array().expect("metrics").len(), 0);
}

#[test]
fn the_market_document_reports_what_the_network_trades() {
    let snapshot = Snapshot::compute(&data(), Coverage::since(JULY), "01J8ZRUN", SEPTEMBER);
    let document = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "market:all")
        .expect("a market window document");
    let payload = serde_json::to_value(document.envelope.payload()).expect("serialises");
    let names: Vec<String> = payload["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .map(|metric| metric["name"].as_str().expect("name").to_string())
        .collect();

    assert!(
        names.iter().any(|name| name.starts_with("market.")),
        "{names:?}"
    );
}

// ---- per-instance documents (§3 `scope`, §6.1.1)

/// Two pubkeys the grammar of §3 can name: 64 lowercase hex.
const ALPHA: &str = "6320ee5e2ce0e1e0ae5d2a3e0b8f1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6fd425";
const BETA: &str = "0f1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8";

/// An order of `pubkey`, in `fiat`.
fn order_by(id: &str, pubkey: &str, fiat: &str, created_at: i64) -> Order {
    let base = order(id, created_at, 10_000);
    Order {
        pubkey: pubkey.to_string(),
        instance: format!("Instance ({})", &pubkey[..8]),
        fiat_code: fiat.to_string(),
        origin: Origin {
            fiat_code: fiat.to_string(),
            ..base.origin.clone()
        },
        ..base
    }
}

/// Two instances: Alpha trades ARS twice and USD once, Beta trades COP once.
fn two_instances() -> Data {
    Data {
        orders: vec![
            order_by("a1", ALPHA, "ARS", AUGUST + DAY),
            order_by("a2", ALPHA, "ARS", AUGUST + 2 * DAY),
            order_by("a3", ALPHA, "USD", AUGUST + 3 * DAY),
            order_by("b1", BETA, "COP", AUGUST + DAY),
        ],
        profiles: vec![profile(ALPHA, "Alpha"), profile(BETA, "Beta")],
        ..Data::default()
    }
}

fn addresses(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .documents
        .iter()
        .map(|document| document.address.to_string())
        .collect()
}

/// The metrics of one document, by address, as `(name, value)`.
fn metrics_of(snapshot: &Snapshot, address: &str) -> Vec<(String, serde_json::Value)> {
    let document = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == address)
        .unwrap_or_else(|| panic!("{address} is missing from {:?}", addresses(snapshot)));
    let payload = serde_json::to_value(document.envelope.payload()).expect("serialises");
    payload["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .map(|metric| {
            (
                metric["name"].as_str().expect("name").to_string(),
                metric["value"].clone(),
            )
        })
        .collect()
}

/// One figure of one document, as a count.
fn count_of(snapshot: &Snapshot, address: &str, name: &str) -> i64 {
    metrics_of(snapshot, address)
        .into_iter()
        .find(|(found, _)| found == name)
        .unwrap_or_else(|| panic!("{address} has no {name}"))
        .1
        .as_i64()
        .unwrap_or_else(|| panic!("{name} of {address} is not a count"))
}

#[test]
fn every_instance_gets_an_orders_document_at_every_window() {
    // Arrange / Act
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);
    let addresses = addresses(&snapshot);

    // Assert — a client that constructs one of these from §3 must not get
    // a miss, exactly as for the network-wide documents.
    for pubkey in [ALPHA, BETA] {
        for window in Span::ALL {
            let address = format!("orders:{}:i:{pubkey}", window.as_str());
            assert!(
                addresses.contains(&address),
                "{address} is missing from {addresses:?}"
            );
        }
    }
}

#[test]
fn an_instance_document_counts_that_instances_orders_and_no_others() {
    // Arrange / Act
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    // Assert — three of the four orders are Alpha's, one is Beta's.
    assert_eq!(count_of(&snapshot, "orders:all", "orders.created"), 4);
    assert_eq!(
        count_of(
            &snapshot,
            &format!("orders:all:i:{ALPHA}"),
            "orders.created"
        ),
        3
    );
    assert_eq!(
        count_of(&snapshot, &format!("orders:all:i:{BETA}"), "orders.created"),
        1
    );
}

#[test]
fn an_instance_document_breaks_its_orders_down_by_currency() {
    // Arrange / Act
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);
    let alpha = format!("orders:all:i:{ALPHA}");

    // Assert — the per-currency counts a frontend renders as
    // "Alpha: ARS 2, USD 1".
    assert_eq!(count_of(&snapshot, &alpha, "orders.ARS.created"), 2);
    assert_eq!(count_of(&snapshot, &alpha, "orders.USD.created"), 1);

    // A currency this instance never traded has no block: absence is not
    // a zero row nobody published.
    let names: Vec<String> = metrics_of(&snapshot, &alpha)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(
        !names.iter().any(|name| name.starts_with("orders.COP.")),
        "{names:?}"
    );
}

#[test]
fn the_per_currency_counts_of_an_instance_add_up_to_its_total() {
    // An order names exactly one currency and belongs to exactly one
    // instance, so the currency blocks partition the instance's orders —
    // which is what lets a client sum them, and sum instances into the
    // network. Payment methods would not: an order names several.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);
    let alpha = format!("orders:all:i:{ALPHA}");

    let per_currency: i64 = metrics_of(&snapshot, &alpha)
        .into_iter()
        .filter(|(name, _)| name.ends_with(".created") && name != "orders.created")
        .map(|(_, value)| value.as_i64().expect("a count"))
        .sum();

    assert_eq!(per_currency, count_of(&snapshot, &alpha, "orders.created"));
}

#[test]
fn an_instance_the_grammar_cannot_name_gets_no_document() {
    // Arrange — a profile whose pubkey is not 64 lowercase hex. It cannot
    // be rendered into a `d` that §3 parses, so publishing one would put a
    // document on a relay under a name no conforming client constructs.
    let data = Data {
        profiles: vec![profile("pk", "Alpha")],
        ..data()
    };

    // Act
    let snapshot = Snapshot::compute(&data, Coverage::since(JULY), "run", SEPTEMBER);

    // Assert
    for address in addresses(&snapshot) {
        assert!(
            !address.contains(":i:"),
            "{address} names an unnameable pubkey"
        );
    }
}

#[test]
fn only_the_orders_report_is_scoped_to_an_instance() {
    // The other seven reports stay network-wide: the cross product of
    // report x window x instance is what §13.4 warns about, and the
    // per-instance figures the other reports would carry are already in
    // `compare` and `instances`, one row per instance.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    for address in addresses(&snapshot) {
        if let Some((report, _)) = address.split_once(':') {
            assert!(
                !address.contains(":i:") || report == "orders",
                "{address} scopes a report that is not published per instance"
            );
        }
        assert!(
            !(address.starts_with("series:") && address.contains(":i:")),
            "{address} is a per-instance series, which §13.4 leaves unpublished"
        );
    }
}

#[test]
fn every_address_a_snapshot_publishes_parses_back() {
    // The round trip of §3 over what is actually published: a document
    // whose `d` a client cannot construct is a document nobody can fetch.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    for document in &snapshot.documents {
        let rendered = document.address.to_string();
        assert_eq!(
            crate::publish::address::Address::parse(&rendered).as_ref(),
            Ok(&document.address),
            "{rendered} does not parse back"
        );
    }
}

// ---- the network's currency counts (§6.1.1)

#[test]
fn the_network_orders_document_counts_orders_by_currency() {
    // Arrange / Act — Alpha traded ARS twice and USD once, Beta COP once.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    // Assert — the mix a client renders without fetching an instance.
    assert_eq!(count_of(&snapshot, "orders:all", "orders.ARS.created"), 2);
    assert_eq!(count_of(&snapshot, "orders:all", "orders.USD.created"), 1);
    assert_eq!(count_of(&snapshot, "orders:all", "orders.COP.created"), 1);
}

#[test]
fn a_currency_count_is_the_sum_of_the_instances_that_traded_it() {
    // The claim §6.1.1 makes to a client: the per-instance documents add
    // up to the network one, because an order has one currency and one
    // instance. Asserted rather than described, since a client that sums
    // them and gets a different number has no way to tell which is wrong.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    for code in ["ARS", "USD", "COP"] {
        let name = format!("orders.{code}.created");
        let per_instance: i64 = [ALPHA, BETA]
            .into_iter()
            .map(|pubkey| {
                metrics_of(&snapshot, &format!("orders:all:i:{pubkey}"))
                    .into_iter()
                    .find(|(found, _)| *found == name)
                    .map_or(0, |(_, value)| value.as_i64().expect("a count"))
            })
            .sum();

        assert_eq!(
            per_instance,
            count_of(&snapshot, "orders:all", &name),
            "{code} does not add up"
        );
    }
}

#[test]
fn the_networks_currency_blocks_carry_counts_and_no_rates() {
    // Four counts per currency, not the nine figures the scoped documents
    // carry. The network's currency list is unbounded and the ceiling of
    // §9.1 is not, so what is published here is the part that sums — and
    // `completion_rate` is `completed / (completed + canceled)`, which a
    // client derives from two of them.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);
    let names: Vec<String> = metrics_of(&snapshot, "orders:all")
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    for suffix in ["created", "completed", "canceled", "open_now"] {
        let name = format!("orders.ARS.{suffix}");
        assert!(names.contains(&name), "{name} is missing from {names:?}");
    }
    for suffix in [
        "completion_rate",
        "abandonment_rate",
        "created_delta",
        "completed_delta",
        "in_progress_now",
    ] {
        let name = format!("orders.ARS.{suffix}");
        assert!(
            !names.contains(&name),
            "{name} should not be published here"
        );
    }

    // The instance-scoped document is the one that carries all nine.
    let scoped: Vec<String> = metrics_of(&snapshot, &format!("orders:all:i:{ALPHA}"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(scoped.contains(&"orders.ARS.completion_rate".to_string()));
}

#[test]
fn the_orders_series_gains_no_column_per_currency() {
    // The window document's currency blocks must not reach the series. A
    // column per currency per bucket is the size problem of §9.1 in the
    // one document shape that already repeats a row per day.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);
    let document = snapshot
        .documents
        .iter()
        .find(|document| document.address.to_string() == "series:orders:daily:2026-08")
        .expect("a daily partition of August");
    let payload = serde_json::to_value(document.envelope.payload()).expect("serialises");

    for column in payload["columns"].as_array().expect("columns") {
        let name = column["name"].as_str().expect("name");
        assert!(
            !["ARS", "USD", "COP"].iter().any(|code| name.contains(code)),
            "{name} plots one currency"
        );
    }
}

#[test]
fn a_currency_untraded_in_the_window_gets_no_block_in_either_scope() {
    // `activity::slice` groups the whole archive in scope, not the
    // window's orders — the deltas need the period before and the `_now`
    // counts need every live order — so a currency whose every order is
    // older than the window still reaches the grouping. Publishing it
    // would put a block of zeros in the document, against the absence
    // rule of §6.3 and against the size argument these blocks exist
    // under: every code the network ever traded would accumulate in
    // `orders:24h` forever.
    //
    // The archive here holds only August, and `24h` is the last day
    // before September.
    let snapshot = Snapshot::compute(&two_instances(), Coverage::since(JULY), "run", SEPTEMBER);

    for address in [
        "orders:24h".to_string(),
        format!("orders:24h:i:{ALPHA}"),
        format!("orders:24h:i:{BETA}"),
    ] {
        let blocks: Vec<String> = metrics_of(&snapshot, &address)
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| ["ARS", "USD", "COP"].iter().any(|code| name.contains(code)))
            .collect();
        assert!(blocks.is_empty(), "{address} carries {blocks:?}");
    }

    // And the window that does hold them still does, so the rule is
    // absence and not silence.
    assert_eq!(count_of(&snapshot, "orders:all", "orders.ARS.created"), 2);
    assert_eq!(
        count_of(
            &snapshot,
            &format!("orders:all:i:{ALPHA}"),
            "orders.ARS.created"
        ),
        2
    );
}
