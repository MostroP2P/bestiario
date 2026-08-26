//! One module per CLI subcommand.
//!
//! Responsibility: wire configuration, database and relays together for a
//! single user-facing operation. Commands hold no domain logic of their own;
//! they assemble the layers below. See `docs/SPEC.md` §10.
