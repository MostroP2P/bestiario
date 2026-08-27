//! A half-open interval of unix seconds: `from <= t < until`.
//!
//! The same shape as the binary's reporting range, defined again here
//! because this crate cannot see that one and an aggregation has to be able
//! to say which window it counted. Half-open so consecutive windows tile:
//! `[Jul, Aug)` then `[Aug, Sep)` counts a boundary event exactly once.

use chrono::{DateTime, Datelike, Months, TimeZone, Utc};

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

    /// The calendar months this window touches, each clipped to it, oldest
    /// first, keyed `YYYY-MM`.
    ///
    /// Clipped rather than whole, so a window opening on the 15th does not
    /// report the first half of that month as if it had been counted.
    pub fn months(&self) -> Vec<(String, Window)> {
        let mut months = Vec::new();
        let Some(start) = DateTime::<Utc>::from_timestamp(self.from, 0) else {
            return months;
        };

        let mut cursor = Utc
            .with_ymd_and_hms(start.year(), start.month(), 1, 0, 0, 0)
            .single()
            .expect("the first of a month at midnight exists");

        while cursor.timestamp() < self.until {
            let next = next_month(cursor);
            let clipped = Window::new(
                cursor.timestamp().max(self.from),
                next.timestamp().min(self.until),
            );
            months.push((cursor.format("%Y-%m").to_string(), clipped));
            cursor = next;
        }

        months
    }
}

fn next_month(at: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = if at.month() == 12 {
        (at.year() + 1, 1)
    } else {
        (at.year(), at.month() + 1)
    };

    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("the first of a month at midnight exists")
}

#[cfg(test)]
mod tests;
