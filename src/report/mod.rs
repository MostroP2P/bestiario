//! Rendering of computed metrics.
//!
//! Responsibility: the two output formats of `docs/SPEC.md` §10 — a
//! `comfy-table` table by default and the `{generated_at, range, metrics}`
//! JSON envelope under `--json` — plus the observed/inferred marking of §5,
//! applied here once so that no individual metric has to remember it.
