//! Slicing a window into periods, and saying which of them the archive can
//! actually speak for.
//!
//! Every `--by period` and `--by day` report walks through here, so the two
//! stay on one convention: UTC calendar buckets, half-open like the window
//! itself, clipped to it, every one of them present.
//!
//! # Why an empty bucket is not the same as an absent one
//!
//! A day nobody traded is a fact, and it is reported as zero. A day before
//! the archive begins is not: relays keep orders for about a fortnight, so
//! a series reaching back past the first backfill would show zeros for days
//! the network was busy — a flat line nobody published, and the most
//! misleading thing a chart of this data could draw. Those buckets report
//! `—` instead, keeping their rows and their names so a consumer sees the
//! shape of the window and the hole in it.

use crate::metric::Metric;
use crate::window::{Period, Window};

/// What the archive holds, as far as a report is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// `created_at` of the earliest stored event; `None` when the archive
    /// holds none, and then it can speak for nothing.
    earliest: Option<i64>,
}

impl Coverage {
    /// The archive reaches back to `earliest`.
    pub const fn since(earliest: i64) -> Self {
        Self {
            earliest: Some(earliest),
        }
    }

    /// From the earliest stored event, if there is one.
    pub fn from_earliest(earliest: Option<i64>) -> Self {
        Self { earliest }
    }

    /// Whether `window` reaches into what the archive holds.
    ///
    /// A window that straddles the first event is covered: part of it is
    /// answerable, and the figures say what was published in that part.
    pub fn covers(&self, window: Window) -> bool {
        self.earliest
            .is_some_and(|earliest| window.until > earliest)
    }

    /// `created_at` of the earliest stored event, if there is one.
    pub fn earliest(&self) -> Option<i64> {
        self.earliest
    }

    /// The series partitions the archive can speak for at `resolution`,
    /// from the one holding its earliest event to the one holding `now`,
    /// oldest first — what a publication run has to compute
    /// (`docs/NOSTR-PUBLICATION.md` §6.3: a partition entirely outside
    /// coverage is no document).
    pub fn partitions(
        &self,
        resolution: crate::publish::address::Resolution,
        now: i64,
    ) -> Vec<crate::publish::address::Bucket> {
        use crate::publish::address::{Bucket, Month, Resolution, Year};
        use chrono::{DateTime, Datelike, Utc};

        let Some(earliest) = self.earliest.filter(|earliest| *earliest <= now) else {
            return Vec::new();
        };
        let (Some(first), Some(last)) = (
            DateTime::<Utc>::from_timestamp(earliest, 0),
            DateTime::<Utc>::from_timestamp(now, 0),
        ) else {
            return Vec::new();
        };

        // A year outside the four digits the grammar can spell has no
        // address, so it has no document either: it is skipped rather
        // than named, which is the same answer `Partition::new` gives.
        match resolution {
            Resolution::Monthly => (first.year()..=last.year())
                .filter_map(Year::new)
                .map(Bucket::Year)
                .collect(),
            Resolution::Daily | Resolution::Weekly => {
                let mut buckets = Vec::new();
                let (mut year, mut month) = (first.year(), first.month());
                while (year, month) <= (last.year(), last.month()) {
                    if let (Some(year), Some(month)) = (Year::new(year), Month::new(month)) {
                        buckets.push(Bucket::Month { year, month });
                    }
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
