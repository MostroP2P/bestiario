//! Slicing a window into periods, and saying which of them the archive can
//! actually speak for.
//!
//! Every `--by period` and `--by day` report walks through here, so the two
//! stay on one convention: UTC calendar buckets, half-open like the window
//! itself, clipped to it, every one of them present.
//!
//! # Why an empty bucket is not the same as an absent one
//!
//! A day nobody traded is a fact, and it is reported as zero. A day
//! outside what the archive holds is not: relays keep orders for about a
//! fortnight, so a series reaching back past the first backfill would show
//! zeros for days the network was busy — a flat line nobody published, and
//! the most misleading thing a chart of this data could draw. Those buckets
//! report `—` instead, keeping their rows and their names so a consumer
//! sees the shape of the window and the hole in it.
//!
//! The rule is the same at the other end, when the extent has one: a day
//! past the archive's last event is a day nobody indexed, and drawing it at
//! zero is the same flat line read the other way round.

use crate::metric::Metric;
use crate::window::{Period, Window};

/// What the archive holds, as far as a report is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// `created_at` of the earliest stored event; `None` when the archive
    /// holds none, and then it can speak for nothing.
    earliest: Option<i64>,
    /// `created_at` of the latest stored event; `None` when the extent
    /// was never read, and then there is no ceiling to enforce.
    ///
    /// Enforced like the floor. `docs/NOSTR-PUBLICATION.md` §6.3 puts
    /// both ends under one rule — a bucket outside `coverage` is `null`
    /// in every column, a partition wholly outside it is no document —
    /// and §5 has a client take that `coverage` from the index. An
    /// archive that published zeros past its own stated ceiling would
    /// contradict the block a client reads to know what the zeros mean,
    /// which is the flat line §6.3 exists to prevent, drawn at the other
    /// end.
    latest: Option<i64>,
}

impl Coverage {
    /// The archive reaches back to `earliest`.
    pub const fn since(earliest: i64) -> Self {
        Self {
            earliest: Some(earliest),
            latest: None,
        }
    }

    /// The archive holds events from `earliest` to `latest`.
    pub const fn between(earliest: i64, latest: i64) -> Self {
        Self {
            earliest: Some(earliest),
            latest: Some(latest),
        }
    }

    /// From the earliest stored event, if there is one.
    pub fn from_earliest(earliest: Option<i64>) -> Self {
        Self {
            earliest,
            latest: None,
        }
    }

    /// From the archive's extent, either end absent when it holds
    /// nothing.
    pub fn from_extent(earliest: Option<i64>, latest: Option<i64>) -> Self {
        Self { earliest, latest }
    }

    /// Whether `window` reaches into what the archive holds.
    ///
    /// Overlap, not containment: a window that straddles either end is
    /// covered, because part of it is answerable and the figures say what
    /// was found in that part. Only a window entirely before the first
    /// event or entirely after the last is not. The window is half-open,
    /// so a window opening exactly on the last event still holds it.
    pub fn covers(&self, window: Window) -> bool {
        self.earliest
            .is_some_and(|earliest| window.until > earliest)
            && self.latest.is_none_or(|latest| window.from <= latest)
    }

    /// `created_at` of the earliest stored event, if there is one.
    pub fn earliest(&self) -> Option<i64> {
        self.earliest
    }

    /// `created_at` of the latest stored event, if the extent was read.
    pub fn latest(&self) -> Option<i64> {
        self.latest
    }

    /// The series partitions the archive can speak for at `resolution`,
    /// from the one holding its earliest event to the one holding the
    /// last of `latest` and `now`, oldest first — what a publication run
    /// has to compute (`docs/NOSTR-PUBLICATION.md` §6.3: a partition
    /// entirely outside coverage is no document).
    ///
    /// The ceiling is the extent's, not the clock's: a run in September
    /// over an archive whose last event is in August has no September
    /// partition to publish, and offering one would put a month of
    /// invented zeros under a `coverage` block that says the archive
    /// stops in August.
    pub fn partitions(
        &self,
        resolution: crate::publish::address::Resolution,
        now: i64,
    ) -> Vec<crate::publish::address::Bucket> {
        use crate::publish::address::{Bucket, Resolution};
        use chrono::{DateTime, Datelike, Utc};

        let Some(earliest) = self.earliest.filter(|earliest| *earliest <= now) else {
            return Vec::new();
        };
        let until = self.latest.map_or(now, |latest| latest.min(now));
        let (Some(first), Some(last)) = (
            DateTime::<Utc>::from_timestamp(earliest, 0),
            DateTime::<Utc>::from_timestamp(until, 0),
        ) else {
            return Vec::new();
        };

        match resolution {
            Resolution::Monthly => (first.year()..=last.year()).map(Bucket::Year).collect(),
            Resolution::Daily | Resolution::Weekly => {
                let mut buckets = Vec::new();
                let (mut year, mut month) = (first.year(), first.month());
                while (year, month) <= (last.year(), last.month()) {
                    buckets.push(Bucket::Month { year, month });
                    (year, month) = if month == 12 {
                        (year + 1, 1)
                    } else {
                        (year, month + 1)
                    };
                }
                buckets
            }
        }
    }
}

/// One bucket per period of `window`, oldest first: `block` computes the
/// figures of the ones the archive can speak for, and the rest carry the
/// same rows with no value.
pub fn walk(
    window: Window,
    period: Period,
    coverage: Coverage,
    block: impl Fn(&str, Window, Option<Window>) -> Vec<Metric>,
) -> Vec<Metric> {
    window
        .buckets(period)
        .into_iter()
        .flat_map(|(key, bucket)| {
            // The bucket before this one, when the archive can speak for
            // it: a Δ against a period nobody indexed is a made-up trend.
            let previous = bucket
                .preceding(period)
                .filter(|previous| coverage.covers(*previous));

            if coverage.covers(bucket) {
                block(&key, bucket, previous)
            } else {
                // The same rows, computed over nothing so the names and
                // kinds are the family's own, with every value withheld.
                block(&key, Window::new(bucket.from, bucket.from), None)
                    .into_iter()
                    .map(Metric::missing)
                    .collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
