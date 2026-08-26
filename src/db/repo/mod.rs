//! One repository per table.
//!
//! Each repository owns the queries for its table and nothing else. Cross-table
//! consistency (a version plus its projection) is the caller's job, inside a
//! transaction.
//!
//! Every function takes an `impl Executor` rather than a `&SqlitePool`, so the
//! same call works against the pool for a one-off write and against a
//! transaction when the pipeline needs several tables to move together
//! (`docs/SPEC.md` §8.1 step 7).

pub mod dev_fees;
pub mod events;
pub mod instances;
pub mod orders;

/// Reports a column SQLite holds but this crate cannot read back.
///
/// Every enum stored as text is written through an `as_str` and read back
/// through its inverse, so a value that fails to convert means the file was
/// written by something other than this crate. That is a decode error, not a
/// panic: it names the column and lets the caller decide.
pub(crate) fn decode<T, E>(column: &'static str, value: Result<T, E>) -> Result<T, sqlx::Error>
where
    E: std::fmt::Display,
{
    value.map_err(|error| sqlx::Error::ColumnDecode {
        index: column.to_string(),
        source: error.to_string().into(),
    })
}
