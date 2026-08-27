# bestiario — Technical Specification

Status: draft v0.1 (2026-08-25). Living document; updated as implementation
progresses.

## 1. Goal

Index the public events that Mostro instances publish on Nostr and produce
global and per-instance statistics for the MostroP2P network: orders by
status, traded volume (sats and fiat), dev fees sent, disputes by status,
most-used fiat currencies and payment methods, and evolution over time.

bestiario does **not** measure reputation: neither of Mostro nodes (that is
`mostro-score`, which rates the reliability of each instance) nor of users
(the `rating` tag on 38383 and kind 38384 are ignored). It measures activity
and volume. It consumes no private data.

## 2. Data sources

All formats were verified against the daemon source (`MostroP2P/mostro`,
`src/nip33.rs`, `src/util.rs`) and `mostro-core 0.14.5`. The spec assumes the
**latest mostrod** (`main` branch); no compatibility with formats or retention
policies of older releases is considered.

### 2.1 Kind 38383 — order (NIP-69, addressable)

Primary source. Order discovery **starts here**: every order bestiario knows
about enters the system through a 38383.

| Tag | Content | Use in bestiario |
|---|---|---|
| `d` | order UUID | natural key of the order |
| `k` | `buy` \| `sell` | direction |
| `f` | fiat code (`ARS`, `USD`, …) | currency |
| `s` | `pending` \| `in-progress` \| `success` \| `canceled` | published status |
| `amt` | sats (0 if market price and not yet taken) | observed volume |
| `fa` | 1 value, or `[min, max]` for a pending range order | fiat volume |
| `pm` | payment methods (one or more values) | payment-method stats |
| `premium` | premium % | premium distribution |
| `network` | `mainnet`, `testnet`, … | filter out test networks |
| `expires_at` | unix ts | order TTL |
| `y` | `[platform, instance_name?]` | **platform filter** + instance name |
| `z` | `order` | discriminator |
| `rating` | maker reputation JSON | ignored (out of scope) |

Notes:

- The event `pubkey` is the Mostro instance. Each instance is a "beast".
- [NIP-69](https://nips.nostr.com/69) defines `y` as the name of the platform
  publishing the order. Mostro adds a **second value with the name of the
  specific node**, because Mostro is not a single node but a network of many
  nodes running the same software: the first value (`mostro`) identifies the
  software — the quickest way to tell that a node is a Mostro node — and the
  second identifies the instance. The second value is optional: an instance
  with no configured name publishes just `["mostro"]`. See §3.
- **Not every NIP-69 order on the Mostro relays is a Mostro order.** A 200-order
  sample of `wss://relay.mostro.network` (2026-08-26, see
  `tests/fixtures/README.md`) carried orders with `y[0]` = `telegram`,
  `hodlhodl`, `Bitblik` and `Bitway` alongside `mostro`. bestiario measures the
  Mostro network, so ingestion filters on `y[0] == "mostro"` (§8.1); without it
  `accept_unknown_instances = true` would fold other platforms into the figures.
- `expires_at` is published by **every** Mostro order in that sample
  (172/172; 172 of the 200 orders overall); the 28 that omitted it all came
  from other platforms. Treat it as mandatory for `y[0] == "mostro"`.
- Real events carry tags this table does not list — `layer`, `expiration`
  (NIP-40), `source`, `name`, `bond`, `reserved_at`, `created_at`, `paid_at`,
  `category`, `taker_fees`. They are not parsed; `events.raw_json` keeps them
  so a later phase can use them without re-capturing.
- Addressable: the relay keeps only the latest version per
  `(pubkey, kind, d)`. bestiario persists **every version** it sees
  (`order_versions`) to reconstruct the lifecycle.
- The daemon publishes only 4 statuses on the wire (`pending`, `in-progress`,
  `success`, `canceled`); internal statuses such as `expired`,
  `canceled-by-admin` or `cooperatively-canceled` arrive collapsed into
  `canceled`, and `dispute`, `fiat-sent`, etc. are not published as such (the
  order stays `in-progress` until resolved). They cannot be told apart from
  the outside; the model states this explicitly.

### 2.2 Kind 8383 — dev fee (regular, non-replaceable)

Published when the instance actually pays the dev fee for a `success` order.
An order may have 0 or 1 (exceptionally >1, see daemon bug #620: all are
stored and flagged as duplicates).

| Tag | Content |
|---|---|
| `order-id` | order UUID (join with 38383 `d`) |
| `amount` | dev fee in sats |
| `hash` | LN payment hash |
| `destination` | lightning address of the development fund |
| `network` | LN network |
| `y` | `["mostro", instance_name?]` |
| `z` | `dev-fee-payment` |

Notes:

- Carries a NIP-40 `expiration` of **1 year** (from the next mostrod release;
  earlier releases used `max_expiration_days`, default 15 days). It is the
  longest-retained record on relays and therefore the most reliable source
  for historical backfill of settled trades: an 8383 may still be available
  when its order's 38383 has already expired. The pipeline must accept
  orphan 8383s and report them as "settled, no order detail". Retention is
  still the relay's decision; bestiario is the archive of record.
- Instances with `fee = 0` never emit it.
- When an 8383 arrives whose order has not been seen yet, it is persisted
  anyway (`dev_fees.order_id` has no hard FK) and reconciled when the 38383
  shows up.

### 2.3 Kind 38386 — dispute (addressable)

| Tag | Content |
|---|---|
| `d` | dispute UUID |
| `s` | `initiated` \| `in-progress` \| `seller-refunded` \| `settled` \| `released` |
| `initiator` | `buyer` \| `seller` |
| `created_at` | unix ts when the dispute was opened (distinct from the event `created_at`) |
| `y`, `z` | instance name, `dispute` |

The event does **not** include the `order-id`. The dispute→order relation is
not observable; disputes are only counted by status, instance and initiator.
Like 38383, every version is persisted.

### 2.4 Kind 38385 — instance info (addressable, `d` = pubkey hex)

Tags used: `fee`, `max_order_amount`, `min_order_amount`,
`fiat_currencies_accepted` (csv), `mostro_version`, `mostro_commit_hash`,
`protocol_version`, `lnd_networks`, `bond_enabled`, `bond_*`, `y`, `z`=`info`.

`protocol_version` is **optional**: 2 of the 20 instances sampled on
2026-08-26 omit it. Real events also carry `expiration_hours`,
`expiration_seconds`, `max_orders_per_response`, `pow`, `pow_first_contact`,
the hold-invoice window/CLTV parameters and a set of `lnd_*` fields; they are
unparsed and kept in `events.raw_json`.

Every version is persisted (history of `fee`, which changes over time; the
value in force *at the time of the order* is needed).

**Does not publish `dev_fee_percentage`.** See §5.

### 2.5 Kind 30078 — exchange rates (addressable, `d` = `mostro-rates`)

Content `{"BTC": {"USD": 50000.0, "ARS": 1.05e8, …}}` = price of 1 BTC in
each currency. Tags `source` (`yadio`), `published_at`, `expiration` (1 h).
One snapshot per event is persisted to value orders at the price of the
moment.

### 2.6 Kind 10002 — relay list (NIP-65)

For each configured pubkey, the relays it publishes to. Used to discover
additional relays (opt-in, see config `discover_relays`).

## 3. Instance identification

Each instance is identified by its **hex pubkey** (primary key). The human
name comes from the second value of the `y` tag of any event (38383, 8383,
38386, 38385) — the Mostro-specific extension to the
[NIP-69](https://nips.nostr.com/69) `y` tag described in §2.1, where the first
value is always the software (`mostro`) and the second the node name. Rules:

- **A third of the network publishes no name.** Of the 22 Mostro instances in
  the 2026-08-26 sample, 8 never send a second value — `y = ["mostro"]` with
  nothing after it — and a ninth sends one on some kinds and not on others. An
  unnamed instance is the normal case, not an edge case: reports must render
  it and `--instance` must resolve it by pubkey alone.
- The same instance may publish its name on one kind and omit it on another
  (`b3626fe9…` names itself on its orders and not on its disputes), so the
  name is taken from whichever event carries it.
- The most recently observed name wins (`instances.name`, `name_seen_at`).
- The name history is also kept in `instance_names` in case an instance is
  renamed.
- Reports show `name (short pubkey)`; if there is no name, only the pubkey.

## 4. Persistence (SQLite, `sqlx 0.9`, migrations in `migrations/`)

Timestamps: unix seconds, `INTEGER`. Amounts: sats `INTEGER`; fiat `REAL`.

```sql
-- Raw event: dedup by id, signature already verified. Source of truth for re-derivation.
CREATE TABLE events (
  id          TEXT PRIMARY KEY,          -- event id hex
  pubkey      TEXT NOT NULL,             -- instance
  kind        INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  d_tag       TEXT,                      -- NULL for 8383
  raw_json    TEXT NOT NULL,
  relay_url   TEXT NOT NULL,             -- first relay it was seen on
  seen_at     INTEGER NOT NULL
);
CREATE INDEX events_kind_created ON events(kind, created_at);
CREATE INDEX events_pubkey_kind_d ON events(pubkey, kind, d_tag);

CREATE TABLE instances (
  pubkey        TEXT PRIMARY KEY,
  name          TEXT,
  name_seen_at  INTEGER,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL
);

CREATE TABLE instance_names (
  pubkey   TEXT NOT NULL,
  name     TEXT NOT NULL,
  seen_at  INTEGER NOT NULL,
  PRIMARY KEY (pubkey, name)
);

-- Every 38385 version seen.
CREATE TABLE instance_info (
  event_id            TEXT PRIMARY KEY REFERENCES events(id),
  pubkey              TEXT NOT NULL,
  created_at          INTEGER NOT NULL,
  fee                 REAL,              -- fraction, e.g. 0.006
  max_order_amount    INTEGER,
  min_order_amount    INTEGER,
  fiat_currencies     TEXT,              -- csv as published
  mostro_version      TEXT,
  protocol_version    TEXT,
  ln_networks         TEXT,
  bond_enabled        INTEGER
);

-- Every 38383 version seen.
CREATE TABLE order_versions (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  order_id     TEXT NOT NULL,            -- d tag
  pubkey       TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  kind         TEXT NOT NULL,            -- buy | sell
  status       TEXT NOT NULL,            -- pending | in-progress | success | canceled
  fiat_code    TEXT NOT NULL,
  amount_sats  INTEGER NOT NULL,
  fiat_amount  REAL,                     -- NULL if range
  fiat_min     REAL,
  fiat_max     REAL,
  payment_methods TEXT NOT NULL,         -- csv
  premium      REAL NOT NULL,
  network      TEXT,
  expires_at   INTEGER
);
CREATE INDEX order_versions_order ON order_versions(order_id, created_at);

-- Projection: latest known state per order (derivable from order_versions).
CREATE TABLE orders (
  order_id        TEXT PRIMARY KEY,
  pubkey          TEXT NOT NULL,
  first_seen_at   INTEGER NOT NULL,      -- created_at of the first version
  last_updated_at INTEGER NOT NULL,
  final_status    TEXT NOT NULL,
  kind            TEXT NOT NULL,
  fiat_code       TEXT NOT NULL,
  amount_sats     INTEGER NOT NULL,      -- from the latest version
  fiat_amount     REAL,
  payment_methods TEXT NOT NULL,
  premium         REAL NOT NULL,
  network         TEXT,
  success_at      INTEGER,               -- created_at of the success version
  canceled_at     INTEGER
);
CREATE INDEX orders_pubkey_status ON orders(pubkey, final_status);
CREATE INDEX orders_success_at ON orders(success_at);

CREATE TABLE dev_fees (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  pubkey       TEXT NOT NULL,
  order_id     TEXT NOT NULL,            -- no FK: may arrive before the order
  amount_sats  INTEGER NOT NULL,
  payment_hash TEXT NOT NULL,
  destination  TEXT,
  network      TEXT,
  created_at   INTEGER NOT NULL,
  is_duplicate INTEGER NOT NULL DEFAULT 0 -- >1 dev fee for the same order
);
CREATE INDEX dev_fees_order ON dev_fees(order_id);

CREATE TABLE dispute_versions (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  dispute_id   TEXT NOT NULL,
  pubkey       TEXT NOT NULL,
  created_at   INTEGER NOT NULL,         -- event created_at
  status       TEXT NOT NULL,
  initiator    TEXT,
  opened_at    INTEGER                   -- created_at tag
);
CREATE INDEX dispute_versions_dispute ON dispute_versions(dispute_id, created_at);

CREATE TABLE disputes (                  -- projection: latest state
  dispute_id   TEXT PRIMARY KEY,
  pubkey       TEXT NOT NULL,
  opened_at    INTEGER,
  last_updated_at INTEGER NOT NULL,
  final_status TEXT NOT NULL,
  initiator    TEXT
);

CREATE TABLE rates (
  event_id     TEXT PRIMARY KEY REFERENCES events(id),
  pubkey       TEXT NOT NULL,
  published_at INTEGER NOT NULL,
  source       TEXT,
  rates_json   TEXT NOT NULL             -- {"USD": 50000.0, ...}
);
CREATE INDEX rates_pubkey_time ON rates(pubkey, published_at);

CREATE TABLE relays (
  url          TEXT PRIMARY KEY,
  source       TEXT NOT NULL,            -- config | nip65:<pubkey>
  first_seen_at INTEGER NOT NULL
);

-- Sync cursor for resumption.
CREATE TABLE sync_state (
  relay_url    TEXT NOT NULL,
  kind         INTEGER NOT NULL,
  last_created_at INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (relay_url, kind)
);
```

`orders` and `disputes` are projections: a `rebuild` command regenerates
them from `*_versions`, and those from `events.raw_json`.

## 5. Observed vs. inferred

| Metric | Type | Source | Error |
|---|---|---|---|
| Orders by status / instance / fiat / method / kind | observed | `orders` | — |
| Sats volume of `success` orders | observed | `orders.amount_sats` where `final_status = success` | — |
| Fiat volume of `success` orders | observed | `orders.fiat_amount` | — |
| Fiat volume converted to USD (or other) | inferred | `amount_sats × rate(pubkey, fiat, ≤ success_at)` | rate up to 5 min old; `rate_age_secs` is reported |
| Total dev fee per instance | observed | `SUM(dev_fees.amount_sats)` excluding duplicates | — |
| Dev fee per order | observed | `dev_fees` join `orders` on `order_id` | — |
| Volume inferred from dev fee | inferred | `dev_fee / (fee_in_force × dev_fee_pct)` | ±1 sat rounding amplified by `1/(fee×pct)`; and `dev_fee_pct` is a config **assumption** (default 0.30) |
| Dev-fee/success ratio | derived | `#success orders with ≥1 dev fee / #success orders` (only instances with `fee > 0`) | — |
| Disputes by status / instance / initiator | observed | `disputes` | — |
| Dispute rate | derived | `#disputes / #orders that left pending` in the window | dispute does not link to order; it is a ratio of counts |

Rule: every inferred figure is reported next to the observed one that
contrasts it, with its error column. The JSON output carries
`"kind": "observed" | "inferred"` per metric.

`dev_fee_percentage` is configured per instance in `settings.toml` with a
global default of `0.30`; if an operator publishes a different value, it is
adjusted there.

Retention asymmetry: with 1-year 8383s and ~15-day 38383s, an initial
backfill will count fewer "success orders" (from 38383) than "settled orders
with dev fee" (from 8383) for instances with `fee > 0`. This is not an
inconsistency; reports show both numbers with their source.

## 6. Metrics catalog

Every metric can be sliced by the **dimensions** in the table and, unless
noted, is offered both globally and per instance. The "each node vs. the
total" comparison is the product's main axis.

| Dimension | Values | Source |
|---|---|---|
| `instance` | pubkey / name | event `pubkey` |
| `period` | day, week, month, year; free range | `created_at` |
| `kind` | `buy`, `sell` | `k` tag |
| `status` | `pending`, `in-progress`, `success`, `canceled` | `s` tag |
| `fiat` | ISO code | `f` tag |
| `payment_method` | values of `pm` | `pm` tag |
| `network` | `mainnet`, `testnet`, … | `network` tag |
| `price_type` | `market` (amt=0 at creation), `fixed` | `amt` of the 1st version |
| `range` | order with `[min,max]` or fixed amount | `fa` of the 1st version |
| `initiator` | `buyer`, `seller` (disputes) | `initiator` tag |

Convention: `∑` = aggregate; `%` = proportion; `p50/p90` = percentiles;
`Δ` = change vs. the previous period. `(inf)` marks inferred metrics.

### 6.1 Activity

| Metric | Definition |
|---|---|
| Orders created | ∑ orders whose 1st version falls in the period |
| Orders completed | ∑ `final_status = success` with `success_at` in the period |
| Orders canceled | ∑ `final_status = canceled` (includes expired) |
| Completion rate | completed / (completed + canceled) |
| No-taker abandonment rate | direct `pending → canceled` / created |
| Open orders now | live `pending` (`expires_at > now`) |
| In-progress orders now | live `in-progress` |
| Δ month over month | growth % of created and completed |
| Activity by hour of day / day of week | histogram of created and completed (UTC) |

### 6.2 Volume

| Metric | Definition |
|---|---|
| Sats volume | ∑ `amount_sats` of completed |
| Fiat volume per currency | ∑ `fiat_amount` of completed, by `fiat` |
| Volume in reference currency (inf) | ∑ `amount_sats × rate(fiat, ≤ success_at)`; reports `rate_age` |
| Average / p50 / p90 ticket | over `amount_sats` and over fiat |
| Size distribution | buckets: <10k, 10k–50k, 50k–200k, 200k–1M, >1M sats |
| Largest order of the period | max `amount_sats` |
| Volume settled via dev fee (inf) | ∑ `dev_fee / (fee × pct)`; shown next to observed with its margin |
| Volume by `kind` | buy / sell split of volume |

### 6.3 Market structure

| Metric | Definition |
|---|---|
| Buy/sell pressure | % of orders (and of volume) `buy` vs `sell`. More `buy` = more BTC demand in that fiat |
| Average / p50 premium | by `fiat` and `kind`, over completed |
| Premium spread | `p50(premium sell) − p50(premium buy)` per fiat |
| Market vs. fixed | % of market-price orders |
| Range orders | % with `[min,max]`; and average range width |
| Fiat ranking | by # orders and by volume; top-3 concentration and HHI |
| Payment-method ranking | by # and volume; per fiat |
| New currencies / methods | first sighting in the period |

### 6.4 Timing

| Metric | Definition |
|---|---|
| Time to fill | `in-progress.created_at − pending.created_at`; p50/p90 by fiat, method and kind |
| Time to complete | `success.created_at − in-progress.created_at` |
| Full cycle | `success.created_at − pending.created_at` |
| Time to cancel | `canceled.created_at − pending.created_at` |
| Book age | average age of live `pending` orders |

### 6.5 Instances (the bestiary)

| Metric | Definition |
|---|---|
| Profile | name, pubkey, `mostro_version`, `protocol_version`, `fee`, limits, accepted fiat, bond policy, LN networks, first/last activity |
| Active instances | with ≥1 order created in the period |
| Market share | % of orders and of volume over the network total |
| Network concentration | HHI over volume share |
| Fee comparison | table of `fee` in force per instance + change history |
| Version adoption | # instances per `mostro_version` over time |
| Fiat coverage | which instances accept each currency |
| Last sign of life | last event of any kind; silent instances > N days |

### 6.6 Dev fees

| Metric | Definition |
|---|---|
| Total sent | ∑ `amount_sats` (no duplicates) |
| Per instance / month | same, sliced |
| Coverage | # `success` with ≥1 dev fee / # `success`, only instances with `fee > 0` |
| Payment latency | `dev_fee.created_at − success_at`, p50/p90 |
| Duplicates detected | # orders with >1 dev fee |
| Orphans | # dev fees without a known 38383 (1-year vs 15-day retention asymmetry) |
| Implied vs. observed | inferred volume (6.2) vs. ∑ `amount_sats` of `success` with dev fee; the difference measures how far the assumed `pct` is from the real one |

### 6.7 Disputes

| Metric | Definition |
|---|---|
| By status | # in `initiated`, `in-progress`, `seller-refunded`, `settled`, `released` |
| By initiator | % opened by buyer vs seller |
| Dispute rate | # disputes opened / # orders that left `pending`, per instance |
| Outcome | % `seller-refunded` vs `settled` vs `released` over resolved |
| Resolution time | `terminal.created_at − opened_at`, p50/p90 |
| Open now and age | non-terminal, sorted by `opened_at` |

### 6.8 Exchange rates

| Metric | Definition |
|---|---|
| Current rate per instance and fiat | latest 30078 |
| Disparity across instances | for the same fiat, `max/min − 1` across instances at the same instant |
| Feed freshness | `now − published_at`; instances with a dead feed |

### 6.9 What cannot be measured (and is stated explicitly)

- Unique users, repeat traders, retention: events carry no user pubkeys.
- Dispute → order: 38386 does not include `order-id`.
- Cancellation reason (expired, admin, cooperative): collapsed into `canceled`.
- Intermediate statuses (`fiat-sent`, `dispute`): arrive as `in-progress`.

### 6.10 Suggested views

Combinations shipped as ready-made reports (CLI and future API):

1. **Network summary** (period): created, completed, rate, sats / reference
   volume, active instances, top fiat, top methods, open disputes.
2. **Instance profile**: 6.5 + its numbers from 6.1/6.2/6.6/6.7 and its share.
3. **Instance comparison**: one row per instance with completed, volume,
   completion rate, fee, dev fee sent, dispute rate, version.
4. **Time series**: any metric from 6.1/6.2/6.6/6.7 by month, global or per
   instance, with Δ.
5. **Market by fiat**: for one currency: buy/sell pressure, premium, methods,
   time to fill, instances that trade it.

## 7. Statuses and lifecycle

Published statuses: `pending → in-progress → success | canceled`, with the
exception `pending → canceled` (expired or canceled without a taker).

On **pending**: it adds nothing to volume, but it is stored anyway (all
versions are persisted already) because it enables metrics that do have
value:

- *Time to fill*: `in-progress.created_at − pending.created_at`, by fiat and
  payment method. Tells which markets have real liquidity.
- *Funnel*: `pending → in-progress` vs `pending → canceled`. How many
  published orders never find a counterparty.
- *Unmet demand*: most requested fiat/methods even if they don't complete.
- *Open book* at a given instant (snapshot of live pending orders).

Implemented in phase 2 (see §6.4); phase 1 only counts current pending
orders.

## 8. Architecture

```
src/
  main.rs            CLI (clap)
  config/            settings.toml → Settings (validated at startup)
  nostr/
    client.rs        relay connections, subscriptions, paginated backfill
    filters.rs       Filter construction by kind/pubkey/since
  ingest/
    mod.rs           pipeline: receive → verify signature → dedup → parse → persist
    parse/           one module per kind: order.rs, dev_fee.rs, dispute.rs, info.rs, rates.rs, relay_list.rs
  db/
    mod.rs           pool, migrations
    repo/            one repository per table (idempotent insert, queries)
  report/            table rendering (comfy-table) and JSON
  commands/          sync, backfill, stats, instances, rebuild
crates/stats/        pure aggregations over structs (no I/O) — testable with fixtures
migrations/          sqlx migrate
tests/fixtures/      real event JSON per kind
```

The aggregation layer receives data already loaded from `db/` and returns
serializable structs; it knows nothing about SQLite or Nostr. It is what an
HTTP API or dashboard reuses later.

It is a **separate workspace crate**, `bestiario-stats`, re-exported as
`bestiario::stats`. A module could keep the no-I/O rule only by convention;
as its own crate, `sqlx`, `nostr-sdk` and `tokio` are not in scope, so code
that reaches for them does not compile.

### 8.1 Ingestion pipeline

1. Receive event from relay.
2. `event.verify()` (signature + id). On failure → discard and log.
3. `pubkey ∈ configured instances` (or `accept_unknown_instances = true`) →
   otherwise discard.
4. For the kinds that carry `y` (38383, 8383, 38386, 38385): `y[0] ==
   "mostro"` → otherwise discard, because the Mostro relays also carry NIP-69
   orders from other platforms (§2.1). 30078 and 10002 publish no `y` at all
   and cannot pass this test.
4b. For those untagged kinds, step 3 alone is not enough when
   `accept_unknown_instances = true`: nothing in a 30078 event says it comes
   from a Mostro instance, and its content sets the price every converted
   figure of §5 is multiplied by. Such an event is taken only from a pubkey
   that is **listed in `instances`**, or that has **already published a
   `y = mostro` event** (a row in `order_versions`, `dev_fees`,
   `dispute_versions` or `instance_info`; the `instances` table itself is no
   proof, since rate events write to it too). Otherwise discard without
   archiving. Kinds are walked with 30078 last, so an instance's tagged
   events are seen before its snapshot is judged; an instance that publishes
   nothing else has to be listed by hand.
5. Filter `network` per config (`networks = ["mainnet"]`) for 38383/8383.
6. Dedup: `INSERT OR IGNORE INTO events`. If it already existed → stop.
7. Parse by kind → insert into the specific table + update the projection
   (`orders`/`disputes`/`instances`) in the same transaction.
8. Advance `sync_state(relay, kind)` to the `max(created_at)` seen.

### 8.2 Backfill and live sync

- `backfill`: per relay and kind, `Filter{kinds, authors, since, until, limit}`
  in windows going backwards until `since = config.backfill_from` or until the
  relay returns empty. For addressable kinds the relay only has the latest
  version; whatever is there is taken.
- `sync`: live subscription with `since = sync_state.last_created_at − overlap`
  (configurable overlap, default 1 h, to tolerate clock skew; dedup absorbs
  the repeats).
- Both share the pipeline in 8.1.

## 9. Configuration (`settings.toml`)

```toml
[nostr]
relays = ["wss://relay.mostro.network", "wss://nos.lol"]
# Discover extra relays via kind 10002 of each instance
discover_relays = true
# Overlap when resuming (seconds)
resume_overlap_secs = 3600

[indexer]
# Instances to follow. Empty + accept_unknown_instances = true → index any
# pubkey that publishes events with y=["mostro", ...].
instances = [
  "82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390",
]
accept_unknown_instances = false
networks = ["mainnet"]
backfill_from = 1735689600          # unix ts; 0 = everything available

[assumptions]
# Share of the fee each instance sends as dev fee. Not published on Nostr;
# daemon default = 0.30.
dev_fee_percentage_default = 0.30
[assumptions.dev_fee_percentage]
# "<pubkey>" = 0.50

[database]
url = "sqlite://bestiario.db"

[report]
# Reference currency for converting fiat volume
reference_currency = "USD"
```

## 10. CLI

```
bestiario backfill [--from TS] [--until TS] [--kind K]
bestiario sync                               # live follow (daemon)

# Views (§6.10). All accept --from/--until/--instance/--network/--json
bestiario summary                            # network summary for the period
bestiario instances                          # the bestiary (profiles)
bestiario instance <PUBKEY|NAME>             # profile + metrics of one instance
bestiario compare                            # one row per instance
bestiario series <metric> [--by month|week|day] [--split instance|kind|fiat]
bestiario market <FIAT>                      # buy/sell pressure, premium, methods, time to fill

# Metric families (§6.1–6.8) with free slicing
bestiario stats orders    [--by status|kind|fiat|method|instance|period|hour|weekday]
bestiario stats volume    [--by kind|fiat|instance|period] [--in USD]
bestiario stats market    [--by fiat|kind|instance]
bestiario stats timing    [--by fiat|method|kind|instance]
bestiario stats dev-fees  [--by instance|period]
bestiario stats disputes  [--by status|initiator|instance|period]
bestiario stats rates     [--fiat F]

bestiario orders <ORDER_ID>                  # lifecycle + dev fee
bestiario rebuild                            # regenerate projections from events
```

Default output is a table; `--json` emits `{ "generated_at", "range",
"metrics": [ { "name", "kind": "observed|inferred", "value", "error": … } ] }`.

## 11. Dependencies (latest versions on crates.io as of 2026-08-25)

| Crate | Version | Use |
|---|---|---|
| `nostr-sdk` | 0.45.2 | relays, filters, event verification |
| `mostro-core` | 0.14.5 | kind constants, order/dispute `Status` |
| `sqlx` | 0.9.0 (`sqlite`, `runtime-tokio`, `migrate`) | persistence |
| `tokio` | 1.53 | runtime |
| `futures` | 0.3.34 | the `Stream` trait behind the live subscription |
| `serde` / `serde_json` | 1.0.229 / 1.0.151 | (de)serialization |
| `toml` | 1.1 | settings |
| `config` | 0.15 | layered loading (file + env) |
| `clap` | 4.6 (`derive`) | CLI |
| `comfy-table` | 8.0 | tables |
| `anyhow` / `thiserror` | 1.0.104 / 2.0.20 | errors |
| `tracing` / `tracing-subscriber` | 0.1.44 / 0.3.23 | logging |
| `chrono` | 0.4.45 | dates in reports |
| `uuid` | 1.25 | order/dispute ids |

## 12. Tests

- **Unit**: per-kind parsers with real event JSON fixtures
  (`tests/fixtures/<kind>/*.json`); `stats/` functions with small synthetic
  datasets and hand-computed expected values (including the dev-fee inverse
  and its error margin).
- **Integration**: ingestion pipeline against in-memory SQLite; idempotency
  (same event twice → one row); projection rebuild; resumption from
  `sync_state`.
- **E2E**: `nostr-sdk` with the `local-relay` feature (as the daemon uses):
  start a local relay, publish fixtures, run `backfill` + `stats --json` and
  compare with the expected result.
- Coverage (`cargo-llvm-cov`) is enforced in two steps:
  - **≥ 80%** from the moment the E2E test exists, which is what makes the
    number mean something. Below that the figure only measures how much
    scaffolding has been written.
  - **≥ 95% overall, and 100% for the pure layers** (`crates/stats` and
    `ingest::parse`) in the hardening phase (§13.4). Those two layers are
    plain functions over plain data with no I/O to stand in the way, so
    anything uncovered there is a case nobody thought about rather than a
    case that is awkward to reach.

  The overall gate stops short of 100% deliberately. The residue is I/O
  failure handling — a relay that drops mid-subscription, a disk that fills
  — where a test costs more to write and maintain than the line is worth.
  What is *not* acceptable is leaving an aggregation or a parser branch
  uncovered, which is why those two are held to 100% separately rather than
  averaged into a single number that can hide them.

## 13. Phases

High-level ordering only. The PR-by-PR breakdown lives in
`docs/ROADMAP.md`, which splits these five into seven numbered phases
(0–6, since the foundations below come before phase 1 here) with
dependencies; when the two disagree, the roadmap is the operational
plan and this section is the intent.

1. **Core**: config, migrations, Nostr client, ingestion of
   38383/8383/38386/38385, projections, `backfill`, `sync`,
   `stats orders|dev-fees|disputes`, `instances`. Table + JSON. Ends with a
   **`README.md`**: what bestiario is, what it can and cannot measure, how to
   install and configure it, and a worked example of every command that
   exists by then. This is the first point at which the tool is usable by
   someone who did not write it, and it does not ship without the document
   that makes that true.
2. **Valuation**: 30078 ingestion, `stats volume --in USD`, inferred vs.
   observed volume, pending metrics (time-to-fill, funnel). The README gains
   the observed/inferred glossary of §5, since from here on some numbers
   carry assumptions and a reader has to be able to tell which.
3. **Discovery**: 10002, `accept_unknown_instances`, monthly series.
4. **Hardening**: raise coverage to the second gate of §12 — ≥ 95% overall
   and 100% across `crates/stats` and `ingest::parse` — and close whatever
   the gap analysis turns up. Deliberately its own phase rather than a
   standing rule: the useful moment to hunt uncovered branches is when the
   metric catalog has stopped moving, and doing it earlier means writing
   tests for code that is about to change.
5. **Exposure**: HTTP API over the aggregation crate (out of this spec's
   scope).

## 14. Open questions

- Is it worth persisting the `rating` tag of 38383 even though it is out of
  scope? Proposal: no; it stays in `events.raw_json` if needed later.
- Dispute rate: since 38386 has no `order-id`, should we propose adding the
  tag to `mostro`? It would be a small upstream change and would make the
  metric observable.
- Instances that publish no name in `y` (a third of the network, §3): keep a
  manual alias in config (`[instances.aliases]`)?
