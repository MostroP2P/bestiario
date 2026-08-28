//! A snapshot in one pass — `docs/NOSTR-PUBLICATION.md` §5–§7.
//!
//! Every document of a publication run, computed from one reading of the
//! archive and sharing one `snapshot_id`: the window documents of §6.1,
//! carrying the SPEC §10 metric records verbatim, and the series partitions
//! of §6.2 in their columnar form. Each comes with the hash of §5, taken
//! over its `payload` alone — the figures, never the clock around them —
//! so that "did this document change" is a question about the answer.
//!
//! # One aggregation, not two
//!
//! A series partition is a series. It goes through [`crate::series`] for
//! every column rather than through a second implementation here, because
//! two ways of computing the same figure are two answers under one heading
//! the moment they drift. What this module adds is the *shape*: the
//! columns declared once, the rows one per bucket, and the absence rule.
//!
//! # Absence
//!
//! `null` is absence and never zero (§6.3). A bucket the archive can speak
//! for that saw nothing has real zeros for its counts and `null` for its
//! rates, exactly as the daily reports print `0` and `—`. A bucket outside
//! coverage is `null` in every column, counts included: a relay keeps
//! orders for about a fortnight, and a series that reached back before the
//! first backfill would otherwise draw a flat line at zero across a period
//! the network was trading — the single most misleading thing this system
//! could publish. A partition entirely outside coverage is no document.
//!
//! The same rule applies at the other end. A partition spans a whole
//! calendar month or year, so the one holding `now` has buckets that have
//! not happened yet; those are `null` in every column too. Publishing
//! zeros for tomorrow would draw the same flat line as publishing zeros
//! for a period nobody indexed, and a chart that dips to zero for the rest
//! of the month is the more convincing of the two lies.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::bucket::Coverage;
use crate::metric::{Metric, MetricKind, Value};
use crate::series::{self, Data};
use crate::window::{Period, Window};

use super::address::{Address, Bucket, Partition as Slot, Report, Resolution, Window as Span};
use super::document::{Envelope, Run, rfc3339};

/// A half-open span as the documents write it: RFC 3339, UTC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Range {
    pub from: String,
    pub until: String,
}

impl Range {
    fn of(window: Window) -> Self {
        Self {
            from: rfc3339(window.from),
            until: rfc3339(window.until),
        }
    }
}

/// The `payload` of a window document (§6.1): the SPEC §10 envelope minus
/// its `generated_at`, which belongs to the run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowPayload {
    pub range: Range,
    pub metrics: Vec<Metric>,
}

/// The payload of a window document over `window`.
pub fn window_payload(window: Window, metrics: &[Metric]) -> WindowPayload {
    WindowPayload {
        range: Range::of(window),
        metrics: metrics.to_vec(),
    }
}

/// One column of a series partition: its name and, declared once rather
/// than once per cell, what kind of figure it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Column {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MetricKind>,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The `payload` of a series partition (§6.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeriesPayload {
    pub period: Range,
    pub resolution: String,
    pub columns: Vec<Column>,
    /// One row per bucket of the period, ascending, none skipped; the
    /// first cell is the bucket's key.
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// A computed series partition: its payload and the hash of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Partition {
    pub address: Address,
    pub period: Window,
    pub payload: SeriesPayload,
    pub hash: String,
}

/// The lowercase hex SHA-256 of `payload`'s deterministic serialisation.
///
/// Deterministic because serde writes struct fields in declaration order
/// and `serde_json` renders every float the shortest way that round-trips,
/// which is also how the SPEC §10 report renders it: the same figures give
/// the same bytes whichever run produced them.
pub fn hash_of(payload: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(payload).expect("a payload of plain data serialises");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The series family behind a report, for the reports that have one.
fn family_of(report: Report) -> Option<series::Family> {
    match report {
        Report::Orders => Some(series::Family::Activity),
        Report::Volume => Some(series::Family::Volume),
        Report::DevFees => Some(series::Family::DevFees),
        Report::Disputes => Some(series::Family::Disputes),
        // Views, and figures without a window shape: no series.
        Report::Summary | Report::Market | Report::Instances | Report::Compare => None,
    }
}

/// The partition of `report` at `resolution` over `bucket`, or `None` when
/// the archive can speak for none of it (§6.3).
pub fn partition(
    data: &Data,
    report: Report,
    resolution: Resolution,
    bucket: Bucket,
    coverage: Coverage,
    now: i64,
) -> Option<Partition> {
    let family = family_of(report)?;
    // The pair is validated once, here: a daily partition of a whole year
    // is not a document, and the address type refuses to name one.
    let slot = Slot::new(resolution, bucket)?;
    let span = slot.window();
    if !coverage.covers(span) {
        return None;
    }

    // The columns are what the family reports over the whole span, minus
    // the figures that have no shape over time — the same rule `series`
    // applies to what it will plot. Taken over the span rather than over
    // one bucket so a column present in some buckets and not others (a
    // currency traded only some days) is declared for all of them.
    let catalogue = series::catalogue_of(family, data, span, now);
    let prefix = format!("{}.", family.prefix());
    let declared = catalogue.iter().map(|name| {
        let sample = series::measure(data, span, now, name);
        Column {
            name: name.strip_prefix(&prefix).unwrap_or(name).to_string(),
            kind: sample.as_ref().map(Metric::kind),
            unit: sample
                .as_ref()
                .map_or_else(|| "missing".to_string(), |metric| unit_of(&metric.value)),
            error: sample
                .as_ref()
                .and_then(|metric| metric.error().map(str::to_string)),
        }
    });
    let columns: Vec<Column> = std::iter::once(Column {
        name: "date".to_string(),
        kind: None,
        unit: "date".to_string(),
        error: None,
    })
    .chain(declared)
    .collect();

    let rows = rows_of(resolution, span)
        .into_iter()
        .map(|(key, slot)| {
            let mut row = Vec::with_capacity(columns.len());
            row.push(serde_json::Value::String(key));
            row.extend(catalogue.iter().map(|name| {
                if !coverage.covers(slot) || slot.from >= now {
                    return serde_json::Value::Null;
                }
                series::measure(data, slot, now, name)
                    .map_or(serde_json::Value::Null, |metric| cell(&metric.value))
            }));
            row
        })
        .collect();

    let payload = SeriesPayload {
        period: Range::of(span),
        resolution: resolution.as_str().to_string(),
        columns,
        rows,
    };
    let hash = hash_of(&payload);

    Some(Partition {
        address: Address::Series {
            report,
            partition: slot,
            scope: None,
        },
        period: span,
        payload,
        hash,
    })
}

/// The buckets a partition has a row for.
///
/// Days and months are the calendar buckets of the span, clipped to it as
/// every report clips them. Weeks are not: a week that straddles a month
/// boundary is filed under the month its first day falls in (§3, and
/// `Bucket::for_week_starting`), so the rows of a weekly partition are the
/// *whole* weeks whose Monday falls inside the month — never clipped, and
/// never repeated in the neighbouring partition.
///
/// Clipping them instead would put `2026-W31` in July's partition as
/// Jul 27–Aug 1 and again in August's as Aug 1–3: one ISO week, one key,
/// two different sets of figures, and a client stitching the months
/// together would double-count the days in between or pick whichever it
/// read last. The cost is that a weekly partition's last row can run a few
/// days past the month it is addressed by, which is what filing a week
/// under one month means.
fn rows_of(resolution: Resolution, span: Window) -> Vec<(String, Window)> {
    let period = match resolution {
        Resolution::Daily => Period::Day,
        Resolution::Monthly => Period::Month,
        Resolution::Weekly => return weeks_of(span),
    };
    span.buckets(period)
}

/// The whole weeks that open inside `span`.
fn weeks_of(span: Window) -> Vec<(String, Window)> {
    // A UTC week is exactly seven days of seconds — there is no daylight
    // saving here to shorten one — so the weeks are stepped as integers
    // and every one of them is representable by construction, with no
    // arithmetic that can fail and no branch for it that no test could
    // reach. 1970-01-01 was a Thursday, so the Monday of its week is
    // three days before the epoch — the shift that puts Mondays on
    // multiples of a week.
    const WEEK: i64 = 7 * 86_400;
    const EPOCH_TO_MONDAY: i64 = 3 * 86_400;

    let monday_of = |at: i64| (at + EPOCH_TO_MONDAY).div_euclid(WEEK) * WEEK - EPOCH_TO_MONDAY;
    let mut monday = monday_of(span.from);
    if monday < span.from {
        // The week the month opens in began in the month before, whose
        // partition already carries it.
        monday += WEEK;
    }

    let mut weeks = Vec::new();
    while monday < span.until {
        let week = Window::new(monday, monday.saturating_add(WEEK));
        // The ISO key is the week's own, spelled by the one place that
        // spells it; a week no calendar can name has no row.
        if let Some((key, _)) = week.weeks().into_iter().next() {
            weeks.push((key, week));
        }
        monday = monday.saturating_add(WEEK);
    }
    weeks
}

/// The `unit` a column declares, from a sample of its figure — the same
/// word the SPEC §10 record carries.
fn unit_of(value: &Value) -> String {
    match value.normalised() {
        Value::Count(_) => "count",
        Value::Sats(_) => "sats",
        Value::Ratio(_) => "ratio",
        Value::Seconds(_) => "seconds",
        Value::Fiat { .. } => "fiat",
        Value::Text(_) => "text",
        Value::Missing => "missing",
    }
    .to_string()
}

/// One cell of a row: the figure alone, its unit and kind being the
/// column's. `null` for a missing figure, matching the `—` of the tables.
fn cell(value: &Value) -> serde_json::Value {
    match value.normalised() {
        Value::Count(count) => serde_json::json!(count),
        Value::Sats(sats) => serde_json::json!(sats),
        Value::Ratio(ratio) => serde_json::json!(ratio),
        Value::Seconds(seconds) => serde_json::json!(seconds),
        Value::Fiat { amount, .. } => serde_json::json!(amount),
        Value::Text(text) => serde_json::json!(text),
        Value::Missing => serde_json::Value::Null,
    }
}

/// One document of a snapshot, ready to be enveloped and signed.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub address: Address,
    pub envelope: Envelope,
    /// The hash of §5, over the payload the envelope carries.
    pub hash: String,
    /// The span a series partition covers; `None` for a window document.
    pub period: Option<Window>,
}

/// Every document of one publication run (§7).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub run: Run,
    pub coverage: Coverage,
    pub documents: Vec<Document>,
}

impl Snapshot {
    /// The whole snapshot from one reading of the archive: every window
    /// document of every report, and every series partition the archive
    /// can speak for, in a stable order. The index (§5) is built from the
    /// result rather than here; the revision is `1` for every document,
    /// since a snapshot on its own knows nothing of the one before it.
    pub fn compute(data: &Data, coverage: Coverage, snapshot_id: &str, now: i64) -> Self {
        let run = Run {
            snapshot_id: snapshot_id.to_string(),
            generated_at: now,
        };
        let mut documents = Vec::new();

        for report in Report::ALL {
            let Some(family) = family_of(report) else {
                continue;
            };
            for span in Span::ALL {
                let window = window_of(span, coverage, now);
                let metrics = series::block_of(family, data, window, now);
                let payload = window_payload(window, &metrics);
                documents.push(Document {
                    address: Address::Window {
                        report,
                        window: span,
                        scope: None,
                    },
                    hash: hash_of(&payload),
                    envelope: Envelope::first(
                        &run,
                        serde_json::to_value(&payload).expect("plain data"),
                    ),
                    period: None,
                });
            }
            for resolution in Resolution::ALL {
                for bucket in coverage.partitions(resolution, now) {
                    if let Some(partition) =
                        partition(data, report, resolution, bucket, coverage, now)
                    {
                        documents.push(Document {
                            address: partition.address,
                            hash: partition.hash,
                            envelope: Envelope::first(
                                &run,
                                serde_json::to_value(&partition.payload).expect("plain data"),
                            ),
                            period: Some(partition.period),
                        });
                    }
                }
            }
        }

        Self {
            run,
            coverage,
            documents,
        }
    }
}

/// The window a `<report>:<window>` document is computed over at `now`.
/// `all` reaches back to the archive's floor, which is what "all" can
/// honestly mean.
fn window_of(span: Span, coverage: Coverage, now: i64) -> Window {
    const DAY: i64 = 86_400;
    let from = match span {
        Span::Hours24 => now - DAY,
        Span::Days7 => now - 7 * DAY,
        Span::Days30 => now - 30 * DAY,
        Span::Days90 => now - 90 * DAY,
        Span::All => coverage.earliest().unwrap_or(now),
    };
    Window::new(from, now)
}

#[cfg(test)]
mod tests;
