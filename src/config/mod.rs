//! Loading and validation of `settings.toml`.
//!
//! Responsibility: turn a configuration file plus `BESTIARIO_*` environment
//! overrides into a validated [`Settings`] value, and reject anything
//! malformed at startup rather than at the point of use. See `docs/SPEC.md`
//! §9 for the file format.
