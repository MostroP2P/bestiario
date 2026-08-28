//! The `d` tag — `docs/NOSTR-PUBLICATION.md` §3.
//!
//! A `d` value is a document's stable name and the only thing a client has
//! to construct to fetch one, so the grammar here is normative: what this
//! parses is what a relay's `#d` filter will find, and nothing else. The
//! rules follow from that. Values are lowercase and match exactly — a
//! client that typed `Summary:30d` gets a miss, not a guess, because a
//! guess would be a document it did not ask for. An instance scope is the
//! whole pubkey, since a prefix is a collision waiting to be found by
//! whoever wants to find it.
//!
//! Parsing and rendering are inverses over the whole grammar: every value
//! that parses renders to the string it came from. That is what lets the
//! type stand in for the string everywhere else.

use std::fmt;

use chrono::{Datelike, NaiveDate};

/// The reports a document can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Report {
    Summary,
    Orders,
    Volume,
    Market,
    Disputes,
    DevFees,
    Instances,
    Compare,
}

impl Report {
    pub const ALL: [Self; 8] = [
        Self::Summary,
        Self::Orders,
        Self::Volume,
        Self::Market,
        Self::Disputes,
        Self::DevFees,
        Self::Instances,
        Self::Compare,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Orders => "orders",
            Self::Volume => "volume",
            Self::Market => "market",
            Self::Disputes => "disputes",
            Self::DevFees => "dev-fees",
            Self::Instances => "instances",
            Self::Compare => "compare",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|report| report.as_str() == text)
    }
}

/// The windows a window document is computed over, relative to the
/// publishing moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Window {
    Hours24,
    Days7,
    Days30,
    Days90,
    All,
}

impl Window {
    pub const ALL: [Self; 5] = [
        Self::Hours24,
        Self::Days7,
        Self::Days30,
        Self::Days90,
        Self::All,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hours24 => "24h",
            Self::Days7 => "7d",
            Self::Days30 => "30d",
            Self::Days90 => "90d",
            Self::All => "all",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|window| window.as_str() == text)
    }
}

/// The bucket size of a series partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    Daily,
    Weekly,
    Monthly,
}

impl Resolution {
    pub const ALL: [Self; 3] = [Self::Daily, Self::Weekly, Self::Monthly];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|resolution| resolution.as_str() == text)
    }
}

/// The span one series partition covers: a month of days or weeks, or a
/// year of months. The shape follows the resolution — a year of days is
/// too large for one document, a year of months is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Month { year: i32, month: u32 },
    Year(i32),
}

impl Bucket {
    /// The partition a week belongs to: the month its first day falls in,
    /// so a week spanning a month boundary lives in one partition only.
    pub fn for_week_starting(monday: NaiveDate) -> Self {
        Self::Month {
            year: monday.year(),
            month: monday.month(),
        }
    }

    /// The span this partition covers: the whole month or year, UTC,
    /// half-open like every window.
    pub fn window(self) -> crate::window::Window {
        use chrono::{TimeZone, Utc};
        let first = |year: i32, month: u32| {
            Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
                .single()
                .map_or(i64::MAX, |at| at.timestamp())
        };
        let (from, until) = match self {
            Self::Month { year, month: 12 } => (first(year, 12), first(year + 1, 1)),
            Self::Month { year, month } => (first(year, month), first(year, month + 1)),
            Self::Year(year) => (first(year, 1), first(year + 1, 1)),
        };
        crate::window::Window::new(from, until)
    }

    /// Whether this bucket has the shape `resolution` calls for.
    fn fits(self, resolution: Resolution) -> bool {
        matches!(
            (self, resolution),
            (Self::Month { .. }, Resolution::Daily | Resolution::Weekly)
                | (Self::Year(_), Resolution::Monthly)
        )
    }

    fn parse(text: &str) -> Option<Self> {
        match text.split_once('-') {
            None => Some(Self::Year(year(text)?)),
            Some((year_text, month_text)) => {
                let month = fixed_digits(month_text, 2)?;
                (1..=12).contains(&month).then_some(Self::Month {
                    year: year(year_text)?,
                    month,
                })
            }
        }
    }
}

impl fmt::Display for Bucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Month { year, month } => write!(f, "{year:04}-{month:02}"),
            Self::Year(year) => write!(f, "{year:04}"),
        }
    }
}

/// A resolution and a bucket that fit each other — the only pair a series
/// address can carry, so that `Display` cannot emit a string `parse`
/// would refuse. Built through [`Partition::new`] alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Partition {
    resolution: Resolution,
    bucket: Bucket,
}

impl Partition {
    /// `bucket` at `resolution`, or `None` when the shapes disagree: a
    /// month of monthly buckets is one bucket, and a year of daily ones is
    /// too large for one document (§3, §9.2).
    pub fn new(resolution: Resolution, bucket: Bucket) -> Option<Self> {
        bucket
            .fits(resolution)
            .then_some(Self { resolution, bucket })
    }

    pub fn resolution(self) -> Resolution {
        self.resolution
    }

    pub fn bucket(self) -> Bucket {
        self.bucket
    }

    /// The span the partition covers.
    pub fn window(self) -> crate::window::Window {
        self.bucket.window()
    }
}

/// What a document is narrowed to, if anything.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// One instance, by its full pubkey in lowercase hex.
    Instance(String),
    /// One network, by the name the indexer uses.
    Network(String),
}

/// The networks a scope may name — the same four the indexer admits.
const NETWORKS: [&str; 4] = ["mainnet", "testnet", "signet", "regtest"];

impl Scope {
    /// Parses the two trailing segments of an address: `i`/`n` and a value.
    fn parse(kind: &str, value: &str) -> Option<Self> {
        match kind {
            "i" => {
                let hex = value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
                hex.then(|| Self::Instance(value.to_string()))
            }
            "n" => NETWORKS
                .contains(&value)
                .then(|| Self::Network(value.to_string())),
            _ => None,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Instance(pubkey) => write!(f, ":i:{pubkey}"),
            Self::Network(network) => write!(f, ":n:{network}"),
        }
    }
}

/// A document's address: the parsed form of its `d` tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Address {
    /// `index`, or `index:<year>` once the index is sharded (§5.1).
    Index { year: Option<i32> },
    /// `<report>:<window>[<scope>]`.
    Window {
        report: Report,
        window: Window,
        scope: Option<Scope>,
    },
    /// `series:<report>:<resolution>:<bucket>[<scope>]`.
    Series {
        report: Report,
        partition: Partition,
        scope: Option<Scope>,
    },
}

/// Why a string is not an address. Names the part that was wrong, since a
/// client author staring at a miss needs to know which segment to fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("an empty string is not a document address")]
    Empty,
    #[error("`{input}`: `{found}` is not a report")]
    Report { input: String, found: String },
    #[error("`{input}`: `{found}` is not a window")]
    Window { input: String, found: String },
    #[error("`{input}`: `{found}` is not a resolution")]
    Resolution { input: String, found: String },
    #[error("`{input}`: `{found}` is not a bucket of a {resolution} partition")]
    Bucket {
        input: String,
        found: String,
        resolution: &'static str,
    },
    #[error("`{input}`: `{found}` is not a year")]
    Year { input: String, found: String },
    #[error("`{input}`: `{found}` is not a scope (`i:<pubkey>` or `n:<network>`)")]
    Scope { input: String, found: String },
    #[error("`{input}`: has {found} segments, and an address of this shape has {expected}")]
    Shape {
        input: String,
        found: usize,
        expected: &'static str,
    },
}

impl Address {
    /// The address `text` names, or why it names none.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        if text.is_empty() {
            return Err(ParseError::Empty);
        }
        let input = text.to_string();
        let segments: Vec<&str> = text.split(':').collect();
        let report_error = |found: &str| ParseError::Report {
            input: input.clone(),
            found: found.to_string(),
        };

        // Exact matching throughout: `split` keeps empty segments, so a
        // trailing or doubled colon shows up as a segment that matches
        // nothing rather than being skipped over. And it yields at least
        // one segment for a non-empty string, which is why the one-segment
        // arm below is the last and there is no empty case to name.
        match segments.as_slice() {
            ["index"] => Ok(Self::Index { year: None }),
            ["index", year_text] => year(year_text)
                .map(|year| Self::Index { year: Some(year) })
                .ok_or_else(|| ParseError::Year {
                    input: input.clone(),
                    found: year_text.to_string(),
                }),

            ["series", report, resolution, bucket, rest @ ..] => {
                let report = Report::parse(report).ok_or_else(|| report_error(report))?;
                let resolution =
                    Resolution::parse(resolution).ok_or_else(|| ParseError::Resolution {
                        input: input.clone(),
                        found: resolution.to_string(),
                    })?;
                let partition = Bucket::parse(bucket)
                    .and_then(|bucket| Partition::new(resolution, bucket))
                    .ok_or_else(|| ParseError::Bucket {
                        input: input.clone(),
                        found: bucket.to_string(),
                        resolution: resolution.as_str(),
                    })?;
                let scope = Self::scope(&input, rest, "4 or 6")?;
                Ok(Self::Series {
                    report,
                    partition,
                    scope,
                })
            }

            [report, window, rest @ ..] => {
                let report = Report::parse(report).ok_or_else(|| report_error(report))?;
                let window = Window::parse(window).ok_or_else(|| ParseError::Window {
                    input: input.clone(),
                    found: window.to_string(),
                })?;
                let scope = Self::scope(&input, rest, "2 or 4")?;
                Ok(Self::Window {
                    report,
                    window,
                    scope,
                })
            }

            // What remains is a single segment: `split` yields at least
            // one for a non-empty string, and every longer shape matched
            // above. A report on its own names no window.
            _ => Err(report_error(text)),
        }
    }

    /// The optional trailing scope of an address, from what followed its
    /// fixed segments.
    fn scope(
        input: &str,
        rest: &[&str],
        expected: &'static str,
    ) -> Result<Option<Scope>, ParseError> {
        match rest {
            [] => Ok(None),
            [kind, value] => Scope::parse(kind, value)
                .map(Some)
                .ok_or_else(|| ParseError::Scope {
                    input: input.to_string(),
                    found: format!("{kind}:{value}"),
                }),
            _ => Err(ParseError::Shape {
                input: input.to_string(),
                found: input.split(':').count(),
                expected,
            }),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index { year: None } => write!(f, "index"),
            Self::Index { year: Some(year) } => write!(f, "index:{year:04}"),
            // One `write!` per shape, the scope folded in as text: a
            // second write behind a `?` would be an error arm that no
            // formatter of a string can reach, and so no test could.
            Self::Window {
                report,
                window,
                scope,
            } => write!(
                f,
                "{}:{}{}",
                report.as_str(),
                window.as_str(),
                rendered(scope)
            ),
            Self::Series {
                report,
                partition,
                scope,
            } => write!(
                f,
                "series:{}:{}:{}{}",
                report.as_str(),
                partition.resolution().as_str(),
                partition.bucket(),
                rendered(scope)
            ),
        }
    }
}

/// The trailing scope segments, or nothing.
fn rendered(scope: &Option<Scope>) -> String {
    scope.as_ref().map(ToString::to_string).unwrap_or_default()
}

/// Exactly `digits` ASCII digits, as a number. Nothing else: not a sign,
/// not whitespace, not one digit fewer.
fn fixed_digits(text: &str, digits: usize) -> Option<u32> {
    (text.len() == digits && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// A four-digit year.
fn year(text: &str) -> Option<i32> {
    fixed_digits(text, 4).map(|year| year as i32)
}

#[cfg(test)]
mod tests;
