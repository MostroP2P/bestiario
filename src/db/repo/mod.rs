//! One repository per table.
//!
//! Each repository owns the queries for its table and nothing else. Cross-table
//! consistency (a version plus its projection) is the caller's job, inside a
//! transaction.
