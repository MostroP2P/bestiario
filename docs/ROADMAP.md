# bestiario — Implementation Roadmap

Status: draft v0.1 (2026-08-25). Companion to `docs/SPEC.md`, which stays the
source of truth for formats, schema and metrics. This document only answers
*in what order do we build it, and what goes in each pull request*.

## Conventions

- **One PR = one row in the tables below.** A PR is mergeable on its own:
  it compiles, `cargo clippy -- -D warnings` is clean, its tests pass, and it
  does not break any command that already worked.
- Every PR that adds behaviour ships its tests **in the same PR** (TDD: test
  first, red, then green). Coverage target ≥ 80% is measured per phase, not
  per PR.
- Commit and PR titles follow conventional commits. The `Title` column is the
  literal PR title.
- **Size** is a rough budget, not a rule: `S` ≤ ~150 lines of diff, `M` ≤ ~400,
  `L` > 400. Anything trending past `L` should be split before opening it.
- **PR numbers are identifiers, not an order.** Merge order is given by the
  `Depends` column alone, and every row's dependencies are listed in full —
  including the ones a reader might assume from adjacency. The numbering is
  kept monotonic with dependencies wherever possible so the two rarely
  disagree.
- `Depends` lists PRs that must be merged first. PRs with no shared dependency
  can be developed in parallel branches.
- Deliberate bundling: some rows group several trivial tasks (e.g. all four
  tag parsers of a kind) because splitting them would produce PRs that cannot
  be reviewed in isolation. Where a row groups tasks, they are listed as
  bullets under `Scope`.

## Phase map

| Phase | Goal | PRs | Exit criterion |
|---|---|---|---|
| 0 | Skeleton that builds, configures and stores | 01–06 | `bestiario --help` runs; migrations apply to an empty DB |
| 1 | Ingestion: relays → SQLite | 07–22 | `backfill` and `sync` populate the order, dev-fee, dispute and instance tables from a real relay; `rebuild` reproduces the projections |
| 2 | Observed statistics and reporting | 23–30 | `summary`, `instances`, `compare` and `stats orders\|dev-fees\|disputes` in table and JSON, covered end to end |
| 3 | Valuation and inference | 31–37 | `stats volume --in USD`, inferred vs. observed volume with error margins |
| 4 | Discovery, series and market views | 38–43 | `series`, `market <FIAT>`, relay/instance discovery |
| 5 | Exposure (HTTP API) | 44–47 | `bestiario-stats` served over HTTP without touching the aggregation layer |

Phases 0–2 produce the first genuinely useful release (`v0.1.0`): counts,
dev fees and disputes, all observed, no inference. Phase 3 is what makes the
numbers comparable across currencies. Phase 4 is breadth. Phase 5 is out of
`SPEC.md` scope and stays optional.

---

## Phase 0 — Foundations

Nothing here talks to a relay. The point is to make every later PR small by
paying the setup cost once.

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 01 | `chore: add dependencies and module skeleton` | S | — | Add every crate from SPEC §11 to `Cargo.toml` at the pinned versions. Create the empty module tree of SPEC §8 (`config`, `nostr`, `ingest`, `db`, `report`, `commands`), each with a `mod.rs` and a doc comment stating its responsibility. Split the crate into a library plus a thin binary so integration tests can drive the CLI's own code paths. Put the aggregation layer in its own workspace crate, `bestiario-stats`, re-exported as `bestiario::stats`: its short dependency list is what makes the no-I/O rule of §8 a compile error rather than a convention. |
| 02 | `ci: build, lint, test and coverage workflow` | S | 01 | GitHub Actions: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo llvm-cov` with a reported (not yet enforced) threshold. Cache the cargo registry. Add `scripts/check-stats-deps.sh`, which checks the dependency tables of `crates/stats` against an allowlist of computation-only crates — the one hole cargo cannot close by itself. |
| 03 | `feat(config): load and validate settings.toml` | M | 01 | <ul><li>`Settings` struct mirroring SPEC §9 with `serde`.</li><li>Layered load (file + `BESTIARIO_*` env) via `config`.</li><li>Validation at startup: relays are valid `wss://` URLs, instance pubkeys are 64 hex chars, `dev_fee_percentage` ∈ (0,1], `networks` non-empty, `reference_currency` is a 3-letter code.</li><li>`settings.toml.example` at the repo root, `settings.toml` in `.gitignore`.</li><li>Tests: a valid file parses; each validation rule has one failing-case test asserting the error message.</li></ul> |
| 04 | `feat(db): connection pool and migrations` | M | 01 | <ul><li>`migrations/0001_initial.sql` with the full schema of SPEC §4, indexes included, generated from the fenced block in the spec so the two cannot drift.</li><li>`db::connect(url: &str)` → `SqlitePool`, `WAL` mode, `foreign_keys = ON`, `synchronous = NORMAL`, `busy_timeout`. A URL rather than a `Settings` value, so persistence has no dependency on configuration.</li><li>`db::migrate(&pool)` running `sqlx::migrate!`.</li><li>Test: migrating an in-memory DB creates every expected table; migrating twice is a no-op.</li></ul> |
| 05 | `feat(cli): command skeleton and logging` | M | 03, 04 | <ul><li>`clap` derive tree for every command in SPEC §10; subcommands return `unimplemented` with a clear message.</li><li>Global flags: `--config`, `--json`, `--from`, `--until`, `--instance`, `--network`, `-v/--verbose`.</li><li>`tracing-subscriber` wired to `-v` and `RUST_LOG`.</li><li>`main` becomes `#[tokio::main]`, loads settings, opens the pool, runs migrations, dispatches.</li><li>Test: `--help` for every subcommand; `--from`/`--until` parse both unix ts and `YYYY-MM-DD`.</li></ul> |
| 06 | `feat(cli): time range and filter resolution` | S | 05 | A `Range { from, until }` resolved once from the global flags (default: last 30 days) plus an `InstanceFilter` resolving `--instance` against pubkey *or* name. Every later stats PR consumes these instead of re-parsing. Tests for the defaulting and the name→pubkey resolution. |

---

## Phase 1 — Ingestion

Order matters here: parsers before repositories before the pipeline, so each
PR can be tested with nothing but fixtures.

### 1a. Fixtures and parsers

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 07 | `test: capture real event fixtures per kind` | S | 01 | `tests/fixtures/{38383,8383,38386,38385,30078,10002}/*.json` with real signed events pulled from a public relay, plus a short `tests/fixtures/README.md` recording where and when each was captured. Include the awkward cases on purpose: a range order (`fa=[min,max]`), a market-price order (`amt=0`), an order with several `pm`, an event with a nameless `y` tag. |
| 08 | `feat(ingest): parse kind 38383 orders` | M | 07 | `parse::order::parse(&Event) -> Result<OrderVersion, ParseError>` per SPEC §2.1. Handles range vs. fixed `fa`, csv `pm`, missing optional tags, unknown `s` values (hard error, not silent default). Tests: one per fixture plus malformed-tag cases. |
| 09 | `feat(ingest): parse kind 8383 dev fees` | S | 07 | `parse::dev_fee::parse` per SPEC §2.2. `order-id`, `amount`, `hash`, `destination`, `network`. Tests including an event missing `destination`. |
| 10 | `feat(ingest): parse kinds 38386 and 38385` | M | 07 | Two parsers, bundled because each is small and they share no logic with anything else: `parse::dispute` (SPEC §2.3, note `created_at` tag ≠ event `created_at`) and `parse::info` (SPEC §2.4, csv `fiat_currencies_accepted`, `fee` as fraction, bond fields). Tests per fixture. |
| 11 | `feat(ingest): extract instance identity from the y tag` | S | 08, 09, 10 | Shared helper `parse::instance_name(&Event) -> Option<String>` reading the second value of `y`, used by all four parsers. Test: `y=["mostro"]` → `None`; `y=["mostro","lnp2pbot"]` → `Some`. |

### 1b. Repositories

Each repository is idempotent (`INSERT OR IGNORE` / upsert) and tested
against an in-memory SQLite created by the phase-0 migrations.

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 12 | `feat(db): events and instances repositories` | M | 04, 11 | `repo::events::insert_if_new -> bool` (the dedup gate of SPEC §8.1 step 5) and `repo::instances::upsert` maintaining `instances` + `instance_names` with the most-recent-name-wins rule of SPEC §3. Tests: same event twice → one row, second call returns `false`; a renamed instance keeps both names in history. |
| 13 | `feat(db): order versions and orders projection` | L | 12, 08 | `repo::orders::insert_version` plus `refresh_projection(order_id)` recomputing the `orders` row from `order_versions` (latest version wins; `success_at`/`canceled_at` from the first version reaching that status; `first_seen_at` from the earliest). Tests: out-of-order arrival still yields the correct projection; a `pending → canceled` order gets no `success_at`. |
| 14 | `feat(db): dev fees repository with duplicate detection` | M | 12, 09 | `repo::dev_fees::insert` flagging `is_duplicate = 1` when the same `order_id` already has a fee (daemon bug #620), keeping the earliest as canonical. No FK on `order_id` — orphans are legal. Tests: two fees for one order → one flagged; an orphan fee inserts cleanly. |
| 15 | `feat(db): disputes and instance info repositories` | M | 12, 10 | `repo::disputes` (versions + latest-state projection, same shape as 13) and `repo::instance_info::insert_version` keeping full `fee` history. Adds `repo::instance_info::fee_in_force(pubkey, at_ts)` — the lookup phase 3 needs. Tests: `fee_in_force` picks the version in force at a past timestamp, not the newest. |

### 1c. Pipeline and commands

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 16 | `feat(db): sync state cursor` | S | 04 | `repo::sync_state` get/advance per `(relay, kind)`. Test: advancing never moves the cursor backwards. |
| 17 | `feat(nostr): filter builders` | S | 01 | `nostr::filters` producing a `Filter` per kind with `authors`, `since`, `until`, `limit`. Pure functions, unit-tested against expected filter JSON — no network. |
| 18 | `feat(nostr): relay client with paginated fetch` | M | 03, 17 | Connect to configured relays, `subscribe`, and `fetch_window(filter)` for the backwards-walking backfill of SPEC §8.2. Handles per-relay failure without aborting the run (log and continue). Tested against the `nostr-sdk` local relay feature. |
| 19 | `feat(ingest): event pipeline` | L | 12–16, 18 | The seven steps of SPEC §8.1 in one place: verify signature → instance allow-list → `network` filter → dedup → parse by kind → persist version + refresh projection **in one transaction** → advance cursor. Returns an `IngestOutcome` enum (`Stored`, `Duplicate`, `Rejected(reason)`) so callers can report counts. Tests: an event with a tampered signature is rejected and stored nowhere; an unknown pubkey is rejected when `accept_unknown_instances = false`; a testnet order is skipped; a valid order lands in `events` + `order_versions` + `orders` + `instances`. |
| 20 | `feat(cmd): backfill` | M | 19 | Backwards windowed walk per relay and kind until `backfill_from` or an empty response; progress logged per window; final summary of stored/duplicate/rejected counts. `--from`, `--until`, `--kind`. Integration test against the local relay seeded with fixtures. |
| 21 | `feat(cmd): sync` | M | 19 | Live subscription with `since = cursor − resume_overlap_secs`, reconnect with backoff, graceful shutdown on SIGINT flushing the cursor. Integration test: publish to the local relay while `sync` runs, assert the row appears. |
| 22 | `feat(cmd): rebuild` | M | 13, 15 | Regenerate `orders` and `disputes` projections from `*_versions`, and optionally (`--from-raw`) regenerate the version tables from `events.raw_json`. Test: wipe both projections, rebuild, assert byte-identical to pre-wipe. |

---

## Phase 2 — Observed statistics

The `bestiario-stats` crate stays I/O-free (SPEC §8): each PR adds a loader in
`db/` returning plain structs, a pure aggregation in `crates/stats`, and a
renderer in `report/`.

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 23 | `feat(report): table and JSON output layer` | M | 06 | The `Metric { name, kind: Observed \| Inferred, value, error }` type of SPEC §5, the `{generated_at, range, metrics}` JSON envelope of SPEC §10, and a `comfy-table` renderer. `(inf)` marking is applied here, once, so no later PR reimplements it. Tests: a mixed observed/inferred set renders both formats as expected. |
| 24 | `feat(stats): activity metrics` | L | 23, 13 | SPEC §6.1: created, completed, canceled, completion rate, abandonment rate, open/in-progress now, Δ vs. previous period, hour-of-day and day-of-week histograms. Sliceable by every dimension of SPEC §6. Wired to `bestiario stats orders --by ...`. Tests over a hand-built dataset with hand-computed expected values. |
| 25 | `feat(stats): dev fee metrics` | M | 23, 14 | SPEC §6.6 minus the inferred volume row (that is PR 33): total, per instance/month, coverage ratio, payment latency p50/p90, duplicates, orphans. Wired to `stats dev-fees`. |
| 26 | `feat(stats): dispute metrics` | M | 23, 15 | SPEC §6.7: by status, by initiator, dispute rate, outcome split, resolution time, currently open with age. Wired to `stats disputes`. |
| 27 | `feat(cmd): instances — the bestiary` | M | 24, 15 | SPEC §6.5 profiles: name, pubkey, versions, fee, limits, accepted fiat, bond policy, first/last activity, silence detection. `bestiario instances` (list) and `bestiario instance <PUBKEY\|NAME>` (profile + that instance's numbers from §6.1/6.6/6.7 + market share). |
| 28 | `feat(cmd): summary` | M | 24, 25, 26 | View 1 of SPEC §6.10: created, completed, rate, sats volume, active instances, top fiat, top methods, open disputes, for the selected range. Table and JSON. |
| 29 | `feat(cmd): compare and orders <ID>` | M | 27, 28 | Bundled: view 3 (one row per instance: completed, volume, completion rate, fee, dev fee sent, dispute rate, version) and the single-order lifecycle view (every version in chronological order + its dev fee). Both are thin assemblies over aggregations that already exist. |
| 30 | `test: end-to-end backfill and stats against a local relay` | M | 20, 28 | SPEC §12 E2E: start the local relay, publish the fixture set, run `backfill` then `summary --json`, compare against a committed expected JSON. Enforce the 80% coverage gate in CI from this PR onward. |

---

## Phase 3 — Valuation and inference

Where the observed/inferred distinction of SPEC §5 starts earning its keep.

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 31 | `feat(ingest): parse and store kind 30078 rates` | M | 19, 20, 21 | Parser per SPEC §2.5, `repo::rates`, wired into the pipeline and into the backfill/sync kind lists. Tests per fixture including a malformed `content`. |
| 32 | `feat(stats): rate lookup with age reporting` | M | 31 | `rate_at(pubkey, fiat, at_ts) -> Option<(rate, age_secs)>` picking the newest snapshot at or before `at_ts`, falling back across instances when the instance has no feed (flagged in the result). This is the single dependency of every converted figure. Tests: exact hit, stale hit with correct age, no rate at all. |
| 33 | `feat(stats): observed volume metrics` | L | 24, 32 | SPEC §6.2 observed rows: sats volume, fiat volume per currency, average/p50/p90 ticket, size-distribution buckets, largest order, volume by kind. `stats volume --by ...`. |
| 34 | `feat(stats): volume in a reference currency` | M | 33 | The inferred conversion row: `amount_sats × rate(fiat, ≤ success_at)`, reported with `rate_age_secs` in the `error` field. `stats volume --in USD`. Tests asserting the error column is populated and the metric is marked `inferred`. |
| 35 | `feat(stats): volume inferred from dev fees` | M | 34, 15, 25 | `dev_fee / (fee_in_force × dev_fee_pct)` per SPEC §5, with the ±1-sat rounding error amplified by `1/(fee×pct)` propagated into `error`, and the implied-vs-observed comparison of SPEC §6.6. Uses `fee_in_force` from PR 15 and the per-instance `dev_fee_percentage` assumption from config. Tests with hand-computed inverses and error bounds. |
| 36 | `feat(stats): market structure metrics` | L | 33 | SPEC §6.3: buy/sell pressure by count and volume, premium average/p50 by fiat and kind, premium spread, market-vs-fixed split, range orders and average width, fiat ranking with top-3 concentration and HHI, payment-method ranking, first sightings. `stats market --by ...`. |
| 37 | `feat(stats): timing and funnel metrics` | L | 24 | SPEC §6.4 and §7: time to fill, time to complete, full cycle, time to cancel (p50/p90 by fiat, method, kind), book age, and the `pending → in-progress` vs `pending → canceled` funnel. Reads `order_versions` directly — this is the payoff for persisting every version. `stats timing --by ...`. |

---

## Phase 4 — Discovery, series and views

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 38 | `feat(stats): exchange rate metrics` | M | 32 | SPEC §6.8: current rate per instance and fiat, cross-instance disparity (`max/min − 1` at the same instant), feed freshness and dead-feed detection. `stats rates --fiat F`. |
| 39 | `feat(nostr): relay discovery via kind 10002` | M | 17 | Parse NIP-65 relay lists for each configured instance, store in `relays` with `source = nip65:<pubkey>`, and add discovered relays to the connection set when `discover_relays = true`. Test: discovery off → relay set unchanged. |
| 40 | `feat(ingest): accept unknown instances` | S | 18 | Honour `accept_unknown_instances = true`: any pubkey publishing events with `y = ["mostro", …]` is indexed and auto-registered in `instances`. Test: an unknown pubkey is rejected with the flag off and stored with it on. |
| 41 | `feat(cmd): series` | L | 33, 24 | `bestiario series <metric> --by month\|week\|day --split instance\|kind\|fiat`, over any metric family from §6.1/6.2/6.6/6.7, with Δ per bucket. Needs a small metric-registry indirection so new metrics become series-able without touching this command. |
| 42 | `feat(cmd): market <FIAT>` | M | 36, 37 | View 5 of SPEC §6.10 for a single currency: buy/sell pressure, premium, methods, time to fill, and which instances trade it. |
| 43 | `docs: README, usage guide and metric glossary` | M | 41 | `README.md` with install, configure, first backfill, and a worked example of every view; a glossary distinguishing observed, inferred and derived per SPEC §5; and the explicit "what cannot be measured" section of SPEC §6.9, so nobody reads a number the wrong way. |

---

## Phase 5 — Exposure (optional, outside SPEC scope)

Only worth starting once phases 2–3 have run against real data for a while and
the metric set has stopped moving.

| # | Title | Size | Depends | Scope |
|---|---|---|---|---|
| 44 | `feat(api): HTTP server skeleton` | M | 43 | `axum`, `/health`, config section `[api]`, graceful shutdown, behind a `api` cargo feature so the CLI-only build stays lean. |
| 45 | `feat(api): metric endpoints over the stats crate` | L | 44 | One endpoint per view of §6.10 plus the `stats` families, reusing the phase-2/3 aggregations unchanged. If any endpoint needs a change inside `crates/stats`, that is a signal the layer boundary was wrong — fix it there, not in the handler. |
| 46 | `feat(api): caching and rate limiting` | M | 45 | Short-TTL response cache keyed by range + filters; per-IP rate limit. Aggregations over a full backfill are not cheap enough to run per request. |
| 47 | `feat(api): OpenAPI schema and JSON contract tests` | M | 45 | Generated schema plus tests asserting the HTTP JSON matches the CLI `--json` envelope exactly — one contract, two transports. |

---

## Sequencing notes

- **Parallelisable clusters.** Within phase 1, PRs 08–11 (parsers) are
  independent of 12–15 (repositories) once 07 lands. Within phase 2, PRs
  24/25/26 are independent of each other once 23 lands. Within phase 3, PRs 36
  and 37 do not depend on each other.
- **The critical path** is 01 → 03 → 04 → 05 → 06 → 12 → 16 → 19 → 20 → 23 →
  24 → 28. Anything
  blocking one of those blocks the first useful release.
- **Do not start phase 3 before PR 30.** The E2E test is what makes it safe to
  add inference on top: without it, an inferred number that is wrong for the
  boring reason (bad ingestion) is indistinguishable from one that is wrong for
  the interesting reason (bad assumption).
- **Open questions from SPEC §14** are deliberately not scheduled. The `rating`
  tag stays in `raw_json`; the upstream `order-id`-on-38386 proposal is an
  issue against `MostroP2P/mostro`, not a PR here; instance aliases become a
  config addition to PR 03 only if a nameless instance actually shows up in
  phase 1.
