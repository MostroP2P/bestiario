# AGENTS.md — bestiario

Instructions for AI coding agents working in this repository.

## Language

**Everything written in this repository is in English**: source code,
comments, doc comments, commit messages, documentation (`docs/`, `README.md`),
CLI help text, log messages, error messages, test names and fixtures.

Conversation with the maintainer may happen in Spanish; the artifacts never do.

## Project

bestiario indexes the public Nostr events published by Mostro instances and
produces network-wide and per-instance statistics. Read `docs/SPEC.md` before
changing anything — it is the source of truth for event formats, the data
model, the metrics catalog and the CLI.

`docs/ROADMAP.md` is the implementation plan: phases, and one row per
pull request. Work is delivered one PR per row.

## Ground rules

- Target the **latest mostrod** (`MostroP2P/mostro`, `main`) and the latest
  published versions of `nostr-sdk`, `mostro-core` and `sqlx`. Do not add
  compatibility shims for older releases.
- Keep the observed / inferred distinction explicit in models and outputs
  (see `docs/SPEC.md` §5).
- Persist every event version; never overwrite history.
- Verify event signatures before persisting anything.
- `stats/` must stay free of I/O so it can back an HTTP API later.
- Tests first (TDD); aggregation functions are tested with real event
  fixtures under `tests/fixtures/`.
- Conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `chore:`, …).
