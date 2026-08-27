//! A half-open interval of unix seconds: `from <= t < until`.
//!
//! The same shape as the binary's reporting range, defined again here
//! because this crate cannot see that one and an aggregation has to be able
//! to say which window it counted. Half-open so consecutive windows tile:
//! `[Jul, Aug)` then `[Aug, Sep)` counts a boundary event exactly once.

use chrono::{DateTime, Datelike, Days, Months, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: i64,
    pub until: i64,
}

impl Window {
    pub fn new(from: i64, until: i64) -> Self {
        Self { from, until }
    }

    pub fn contains(&self, timestamp: i64) -> bool {
        timestamp >= self.from && timestamp < self.until
    }

    /// The window of the same length ending where this one starts — what a
    /// "Δ vs. the previous period" compares against.
    pub fn previous(&self) -> Self {
        let length = self.until - self.from;
        Self {
            from: self.from - length,
            until: self.from,
        }
    }

    /// The same span one calendar month earlier — what a month's "Δ month
    /// over month" compares against.
    ///
    /// Not [`previous`](Self::previous): months are not all the same length,
    /// and a March compared against the thirty-one days before it would be
    /// compared against three days of January. Both bounds move back by one
    /// calendar month, so a whole month is compared against the whole month
    /// before it, and a month clipped by the reporting window against the
    /// same days of the month before. A day the earlier month does not have
    /// clamps to its last one.
    ///
    /// `None` only for a bound outside the range a date can represent.
    pub fn previous_month(&self) -> Option<Self> {
        let back = |timestamp: i64| {
            DateTime::<Utc>::from_timestamp(timestamp, 0)?
                .checked_sub_months(Months::new(1))
                .map(|at| at.timestamp())
        };

        Some(Self::new(back(self.from)?, back(self.until)?))
    }

    /// The buckets of `period` this window touches, each clipped to it,
    /// oldest first — what a series is plotted over.
    pub fn buckets(&self, period: Period) -> Vec<(String, Window)> {
        self.buckets_upto(period, usize::MAX)
            .expect("a window a date can represent has fewer buckets than `usize::MAX`")
    }

    /// The same, refusing rather than building more than `limit` of them.
    ///
    /// A window is as wide as the caller typed, and `--from 0 --until
    /// 9223372036854775807 --by day` asks for a hundred trillion buckets.
    /// Building them all to discover there are too many spends the memory
    /// the limit exists to protect, and walks the cursor past the last date
    /// `chrono` can represent on the way. So the walk stops one bucket past
    /// the limit: enough to know it was exceeded, and no more.
    pub fn buckets_upto(&self, period: Period, limit: usize) -> Option<Vec<(String, Window)>> {
        match period {
            Period::Day => self.walk(period, "%Y-%m-%d", limit, |at| {
                at.checked_add_days(Days::new(1))
            }),
            Period::Week => self.walk(period, "%G-W%V", limit, |at| {
                at.checked_add_days(Days::new(7))
            }),
            Period::Month => self.walk(period, "%Y-%m", limit, next_month),
            Period::Year => self.walk(period, "%Y", limit, |at| {
                Utc.with_ymd_and_hms(at.year() + 1, 1, 1, 0, 0, 0).single()
            }),
        }
    }

    /// The calendar days this window touches, keyed `YYYY-MM-DD`.
    pub fn days(&self) -> Vec<(String, Window)> {
        self.buckets(Period::Day)
    }

    /// The ISO weeks this window touches, keyed `YYYY-Www` and starting on
    /// Monday, which is where an ISO week starts.
    pub fn weeks(&self) -> Vec<(String, Window)> {
        self.buckets(Period::Week)
    }

    /// The calendar years this window touches, keyed `YYYY`.
    pub fn years(&self) -> Vec<(String, Window)> {
        self.buckets(Period::Year)
    }

    /// The calendar months this window touches, each clipped to it, oldest
    /// first, keyed `YYYY-MM`.
    ///
    /// Clipped rather than whole, so a window opening on the 15th does not
    /// report the first half of that month as if it had been counted.
    pub fn months(&self) -> Vec<(String, Window)> {
        self.buckets(Period::Month)
    }
}

impl Window {
    /// The instant the bucket containing `self.from` opens — a midnight, a
    /// Monday, a first of the month or a new year — or `None` for a bound
    /// outside the range a date can represent.
    fn opening(&self, period: Period) -> Option<DateTime<Utc>> {
        let start = DateTime::<Utc>::from_timestamp(self.from, 0)?;
        let midnight = |date: chrono::NaiveDate| date.and_hms_opt(0, 0, 0).map(|at| at.and_utc());

        match period {
            Period::Day => midnight(start.date_naive()),
            Period::Week => midnight(
                start.date_naive() - Days::new(start.weekday().num_days_from_monday() as u64),
            ),
            Period::Month => Utc
                .with_ymd_and_hms(start.year(), start.month(), 1, 0, 0, 0)
                .single(),
            Period::Year => Utc.with_ymd_and_hms(start.year(), 1, 1, 0, 0, 0).single(),
        }
    }

    /// Walks whole buckets from `opening`, clipping each to this window and
    /// keying it by `format`. A bucket the window does not actually reach
    /// into — the part of the first one before it opens — is not a bucket.
    ///
    /// `None` once more than `limit` buckets have been built, and once the
    /// cursor steps past the last instant a date can represent: a window
    /// that wide has more buckets than any caller asked for, and the two
    /// are the same answer.
    fn walk(
        &self,
        period: Period,
        format: &str,
        limit: usize,
        next: impl Fn(DateTime<Utc>) -> Option<DateTime<Utc>>,
    ) -> Option<Vec<(String, Window)>> {
        let mut buckets = Vec::new();
        let Some(mut cursor) = self.opening(period) else {
            return Some(buckets);
        };

        while cursor.timestamp() < self.until {
            let following = next(cursor)?;
            let clipped = Window::new(
                cursor.timestamp().max(self.from),
                following.timestamp().min(self.until),
            );
            if clipped.from < clipped.until {
                if buckets.len() == limit {
                    return None;
                }
                buckets.push((cursor.format(format).to_string(), clipped));
            }
            cursor = following;
        }

        Some(buckets)
    }
}

/// The size of a series bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

/// The first of the month after `at`, or `None` past the last year a date
/// can represent.
fn next_month(at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (year, month) = if at.month() == 12 {
        (at.year() + 1, 1)
    } else {
        (at.year(), at.month() + 1)
    };

    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()
}

#[cfg(test)]
mod tests;
