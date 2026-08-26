//! The reporting window and the instance filter, resolved once.
//!
//! Every statistics command takes `--from`, `--until`, `--instance` and
//! `--network`. Resolving them in one place means the questions they raise —
//! which end of the window is inclusive, what a missing bound means, how a
//! name becomes a pubkey — are answered once, the same way for every command,
//! and are testable without a command around them.

use chrono::{DateTime, Utc};

/// How far back a report reaches when the user gives no `--from`.
const DEFAULT_WINDOW_DAYS: i64 = 30;

/// A half-open interval of unix seconds: `from <= t < until`.
///
/// Half-open rather than inclusive on both ends, so that consecutive windows
/// tile without overlapping. `[Jan, Feb)` followed by `[Feb, Mar)` counts every
/// event exactly once; two inclusive windows would count anything at the
/// boundary twice, and a monthly series is the main consumer here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    from: i64,
    until: i64,
}

/// What went wrong resolving a window.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RangeError {
    #[error("the reporting window is empty: --from {from} is not before --until {until}")]
    Empty { from: i64, until: i64 },
}

impl Range {
    /// Resolves the window from the two optional bounds and the current time.
    ///
    /// `now` is a parameter rather than a call to [`Utc::now`] so that the
    /// defaulting is testable without the answer changing every day.
    pub fn resolve(from: Option<i64>, until: Option<i64>, now: i64) -> Result<Self, RangeError> {
        // A missing `--until` means "up to now", not "up to the last event
        // stored": a report covering a quiet week should say zero, not
        // silently shrink its window to the last thing that happened.
        let until = until.unwrap_or(now);
        let from = from.unwrap_or(until - DEFAULT_WINDOW_DAYS * 86_400);

        if from >= until {
            return Err(RangeError::Empty { from, until });
        }

        Ok(Self { from, until })
    }

    /// The whole of recorded time, for commands that are not windowed.
    pub fn unbounded() -> Self {
        Self {
            from: 0,
            until: i64::MAX,
        }
    }

    /// Inclusive lower bound.
    pub fn from(&self) -> i64 {
        self.from
    }

    /// Exclusive upper bound.
    pub fn until(&self) -> i64 {
        self.until
    }

    /// Whether a unix timestamp falls inside the window.
    pub fn contains(&self, timestamp: i64) -> bool {
        timestamp >= self.from && timestamp < self.until
    }

    /// The window of the same length ending where this one starts, which is
    /// what the "Δ vs. the previous period" metrics of SPEC §6.1 compare
    /// against.
    pub fn previous(&self) -> Self {
        let length = self.until - self.from;
        Self {
            from: self.from - length,
            until: self.from,
        }
    }

    /// The window as it should appear in the `range` field of the JSON
    /// envelope (SPEC §10).
    pub fn to_rfc3339(&self) -> (String, String) {
        (format_timestamp(self.from), format_timestamp(self.until))
    }
}

fn format_timestamp(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

/// Which instances a report covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceFilter {
    /// Every instance in the database.
    All,
    /// One instance, already resolved to its pubkey.
    One { pubkey: String },
}

/// Why an instance could not be resolved.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstanceError {
    #[error("no instance matches `{needle}`")]
    NotFound { needle: String },

    #[error(
        "`{needle}` matches {} instances: {}. Use a pubkey to disambiguate.",
        pubkeys.len(),
        pubkeys.join(", ")
    )]
    Ambiguous {
        needle: String,
        pubkeys: Vec<String>,
    },
}

/// A `(pubkey, name)` pair as loaded from the `instances` table.
pub type KnownInstance = (String, Option<String>);

impl InstanceFilter {
    /// Resolves `--instance` against the instances the database knows about,
    /// in a fixed order of precedence:
    ///
    /// 1. **An exact pubkey.** Unique by construction — it is the primary key
    ///    — so it is never ambiguous and wins outright.
    /// 2. **An exact name.** Names come from the free-text `y` tag and the
    ///    `instances.name` column carries no uniqueness constraint, so two
    ///    instances may well advertise the same one. Several exact name
    ///    matches are therefore ambiguous, not a race to be first.
    /// 3. **A pubkey prefix**, if it identifies exactly one instance.
    ///
    /// Ambiguity at any level is an error naming the candidates rather than a
    /// silent pick: reporting on the wrong instance is worse than being asked
    /// for another character.
    pub fn resolve(needle: Option<&str>, known: &[KnownInstance]) -> Result<Self, InstanceError> {
        let Some(needle) = needle else {
            return Ok(Self::All);
        };
        let needle = needle.trim();

        // Pubkeys are lowercase hex, so ASCII folding is both correct and
        // exact for them.
        if let Some((pubkey, _)) = known.iter().find(|(p, _)| p.eq_ignore_ascii_case(needle)) {
            return Ok(Self::One {
                pubkey: pubkey.clone(),
            });
        }

        // Names are arbitrary text, so folding has to be Unicode-aware:
        // `eq_ignore_ascii_case` would leave `Möstro` and `MÖSTRO` as
        // different names while promising case-insensitive matching.
        let folded = needle.to_lowercase();

        let by_name = matching(known, |(_, name)| {
            name.as_deref().is_some_and(|n| n.to_lowercase() == folded)
        });
        if let Some(resolved) = one_of(needle, &by_name)? {
            return Ok(resolved);
        }

        let by_prefix = matching(known, |(pubkey, _)| {
            pubkey.to_lowercase().starts_with(&folded)
        });
        one_of(needle, &by_prefix)?.ok_or(InstanceError::NotFound {
            needle: needle.to_string(),
        })
    }
}

fn matching(known: &[KnownInstance], predicate: impl Fn(&KnownInstance) -> bool) -> Vec<String> {
    known
        .iter()
        .filter(|instance| predicate(instance))
        .map(|(pubkey, _)| pubkey.clone())
        .collect()
}

/// Collapses a set of candidates into at most one instance: none is "keep
/// looking", one resolves, and more than one is an error rather than a choice.
fn one_of(needle: &str, candidates: &[String]) -> Result<Option<InstanceFilter>, InstanceError> {
    match candidates {
        [] => Ok(None),
        [pubkey] => Ok(Some(InstanceFilter::One {
            pubkey: pubkey.clone(),
        })),
        several => Err(InstanceError::Ambiguous {
            needle: needle.to_string(),
            pubkeys: several.to_vec(),
        }),
    }
}

#[cfg(test)]
mod tests;
