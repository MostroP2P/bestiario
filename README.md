<div align="center">
  <img src="assets/logo.png" alt="bestiario" width="360">
</div>

# bestiario

Statistics for the [Mostro](https://mostro.network) network, computed from
the public Nostr events its instances publish.

Every Mostro instance announces its orders, dev fees, disputes and its own
configuration as signed Nostr events on public relays. bestiario reads
those events, keeps every version of every one of them in a local SQLite
database, and turns them into figures: how many orders were created and
completed, in which currencies, by which instance, how long disputes take,
how much each instance forwards to the development fund. Network-wide, and
one instance against the rest — the bestiary.

It is a command-line tool. Every report prints as a table by default and as
JSON with `--json`.

## What it cannot measure

Stated first, because a number that is not there is easy to mistake for a
number that is zero. The events carry no more than they carry:

- **Unique users, repeat traders, retention.** Orders and disputes are
  published by the instance, not by the people trading. No user pubkey is
  anywhere in the data.
- **Which order a dispute is about.** A dispute event (kind 38386) does not
  name its order. Disputes are counted by status, initiator and instance,
  and the *dispute rate* is a ratio of two counts — disputes opened over
  orders that found a taker — never a pairing.
- **Why an order was canceled.** Expired, canceled by an admin, canceled
  cooperatively: all arrive as `canceled`.
- **The intermediate states.** `fiat-sent` and `dispute` are published as
  `in-progress`; only `pending`, `in-progress`, `success` and `canceled`
  reach the wire.
- **Anything that needs a price, exactly.** Volume in a reference currency
  rests on the rate an instance published minutes before a trade, and the
  volume implied by dev fees rests on an assumed fee share. Those are
  *inferred* figures: marked `(inf)` in every table and `"kind": "inferred"`
  in every JSON record, with what qualifies them in an `error` column
  beside them. Everything else below is a count of published events.

## Install

A Rust toolchain (stable, edition 2024) is the only requirement; SQLite is
bundled.

```sh
git clone https://github.com/MostroP2P/bestiario
cd bestiario
cargo install --path .
```

Or `cargo build --release` and run `target/release/bestiario`.

## Configure

```sh
cp settings.toml.example settings.toml
```

`settings.toml.example` documents every key. The ones that matter first:

| Key | What it does |
|---|---|
| `[nostr].relays` | Relays to read from. `wss://relay.mostro.network` carries every instance. |
| `[nostr].discover_relays` | `true` to also dial the relays the instances say they publish to (NIP-65, kind 10002). Off by default. |
| `[indexer].instances` | Pubkeys to follow, as hex or `npub1…`. |
| `[indexer].accept_unknown_instances` | `true` to index every pubkey that publishes Mostro events, whether listed or not. Events from other platforms on the same relays (they exist) are turned away either way. |
| `[indexer].networks` | Which networks count: `mainnet` alone by default. |
| `[indexer].backfill_from` | How far back the first `backfill` reaches, as a unix timestamp. |
| `[database].url` | Where the archive lives: `sqlite://bestiario.db`. |

`bestiario` reads `settings.toml` from the current directory; `--config`
points it elsewhere. Every value is validated at startup, and a bad one is
an error naming the key rather than a report with a hole in it.

### Relay discovery

Each instance publishes a NIP-65 relay list saying where it reads and where
it writes. bestiario records the relays it *writes* to — an instance's
events are only fetchable where it publishes them, and a relay it merely
reads from holds nothing of its own — and, with `discover_relays = true`,
dials them alongside the configured ones.

Discovery is additive and never subtractive: the configured relays always
come first and are never dropped, since they are the operator's decision
while a discovered relay is a third party's claim. Only the relays an
instance says it *writes* to are taken, which NIP-65 spells as an `r` tag
with no marker or with `write`; an entry marked anything else carries no
such claim and is dropped, like a URL that cannot be dialled. With the flag
off the connection set is exactly what `settings.toml` lists, whatever the
instances have advertised. A relay list carries no `y` tag, so bestiario
takes one only from a pubkey it already knows as an instance — the same
rule that guards rate snapshots — and it *asks* only about those pubkeys
too, even with `accept_unknown_instances = true`. Kind 10002 is the kind
every Nostr user publishes: requesting it of no author in particular would
download the network's whole NIP-65 index to throw all but a handful of it
away. Until something has vouched for a publisher, the two untagged kinds
are not requested at all.

The connection set is not fixed at startup. A relay list read during a
`backfill` is followed by that same invocation — the run walks the relays it
discovers rather than leaving them for a second one — and a `sync` that
stores one rebuilds its subscription over the wider set there and then, so a
process meant to run for months does not go on dialling only the relays it
happened to know about on the day it started.

## First backfill

```console
$ bestiario backfill
backfill: 21 stored, 5 already known, 3 rejected
```

`backfill` walks each relay's history backwards, from now down to
`backfill_from`, one kind at a time, and stores what it finds. It prints a
summary of what was stored, what it had already seen and what it turned
away, and it is safe to run again: nothing is written twice. Relays keep
dev fees for about a year and orders for about a fortnight, so on a fresh
archive expect fees for orders you will never see; the reports call those
*orphans*.

Every report needs this step: over a database that holds no events yet,
a report refuses to run and says so, rather than printing a table of
zeros that reads like an answer. The database path in `settings.toml` is
relative to the directory you run from, so a backfill and a report have
to be run from the same place.

To keep the archive current afterwards:

```sh
bestiario sync
```

`sync` subscribes to the relays and stores events as they arrive, until
interrupted. It resumes from where it left off.

## Reports

Every windowed report below — all of them except `orders <ORDER_ID>`,
which shows one order whole — accepts the same window and scope flags:

- `--from` and `--until`, as a unix timestamp or `YYYY-MM-DD` (UTC). The
  window is half-open — `--until` is excluded — so consecutive windows tile.
  The default is the last thirty days, and a window wider than a century is
  refused: further back than any relay's history and further forward than
  any question about it, so it is a typo rather than a request.
- `--instance <PUBKEY|NAME>`: one instance, by pubkey, by a unique prefix of
  it, or by name.
- `--network <NETWORK>`: one network, overriding the configured list.
  Dispute events carry no network tag, so disputes cannot be narrowed this
  way: `stats disputes` refuses the flag, and the views that include a
  dispute figure (`summary`, `instance`, `compare`) report it as `—` under
  it rather than as a network-wide number beneath a network-scoped heading.
- `--json`.

The examples are real output, captured by the test suite from the corpus of
signed events under `tests/fixtures/` — every event there is one an instance
actually published. That corpus was chosen to cover the shapes a parser has
to survive, not to look like a month of trading: eight orders and five
disputes from unrelated instances make for a dispute rate nobody should
read as a market figure. The shape of each report is what to look at.

Every command shown is executed by `tests/e2e.rs` against that corpus, on
a frozen clock, and the output shown under it has to be what the binary
printed — byte for byte — so an example that has drifted fails the build.
Two things follow from how that suite works. The relay it seeds refuses
expired events, as any relay does, so the corpus is re-signed for it with
keys derived from each instance's real pubkey: the pubkeys in the examples
are those test keys, not the instances' own. And the clock is frozen at
2026-08-27T03:06:40Z, which is why five orders are still "open now".

## What a number rests on

Every figure in every report is one of three things, and the difference is
the difference between a measurement and an estimate. It is marked, not
explained in a footnote: an inferred figure carries `(inf)` after its name
in a table and `"kind": "inferred"` in JSON, with what qualifies it in the
`error` column beside it.

| | What it is | How to read it |
|---|---|---|
| **Observed** | A count or a sum of published events. `orders.created`, `volume.sats`, `dev_fees.total_sats`, `disputes.opened`. | A fact about what the instances published. If it is wrong, either an event was missed or the parser is. |
| **Derived** | Arithmetic over observed figures: a rate, a share, a percentile. `orders.completion_rate`, `market.buy_orders_share`, `timing.time_to_fill_p50`, `disputes.rate`. | As solid as the two numbers under it, and no more. Read the definition before the value — `disputes.rate` divides disputes by orders that found a taker, and pairs no dispute to any order, because a dispute event does not name one. |
| **Inferred** | Rests on something nobody published, so it cannot be checked against the wire. Marked `(inf)`. | Never quote it as a measurement. The `error` column states the assumption and the margin; if you disagree with the assumption, the figure is yours to recompute. |

Only two families are inferred, and both say why:

- **Volume in a reference currency** (`stats volume --in USD`) multiplies
  each completed order by the rate its instance had published by the moment
  it settled. The assumption is that the rate a few minutes old still held
  when the trade closed; the `error` column gives the age of the oldest
  quote used, how many orders were priced on another instance's snapshot,
  and how many could not be priced and are excluded from the sum.
- **Volume implied by dev fees** (`stats dev-fees`) inverts each fee by
  `fee_in_force × dev_fee_percentage`. That percentage is *not published by
  any instance* — it is the `[assumptions]` key of `settings.toml`, `0.30`
  by default — and one satoshi of rounding on a fee is `1 ÷ (fee × pct)`
  satoshis of volume. Both are in the `error` column, and
  `implied_vs_observed` sets the figure against the observed volume of the
  orders that are still known, which is the measure of how far the assumed
  share is from the real one.

Everything else in this README is observed or derived. Where a figure
cannot be computed at all, it reads `—` in a table and `null` in JSON —
never zero: zero is an answer, and the absence of one is not.

### Network summary

```console
$ bestiario summary --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌──────────────────────────┬───────────────────────────────────────────┐
│ metric                   ┆ value                                     │
╞══════════════════════════╪═══════════════════════════════════════════╡
│ summary.created          ┆ 8                                         │
│ summary.completed        ┆ 1                                         │
│ summary.completion_rate  ┆ 50.0%                                     │
│ summary.volume_sats      ┆ 1361 sats                                 │
│ summary.active_instances ┆ 4                                         │
│ summary.top_fiat         ┆ ARS (2), BRL (2), EUR (2)                 │
│ summary.top_methods      ┆ CBU (2), CVU (2), BBVA Efectivo Móvil (1) │
│ summary.open_disputes    ┆ 2                                         │
└──────────────────────────┴───────────────────────────────────────────┘
```

### Activity

```console
$ bestiario stats orders --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────┬───────┐
│ metric                  ┆ value │
╞═════════════════════════╪═══════╡
│ orders.created          ┆ 8     │
│ orders.completed        ┆ 1     │
│ orders.canceled         ┆ 1     │
│ orders.completion_rate  ┆ 50.0% │
│ orders.abandonment_rate ┆ 12.5% │
│ orders.created_delta    ┆ —     │
│ orders.completed_delta  ┆ —     │
│ orders.open_now         ┆ 4     │
│ orders.in_progress_now  ┆ 1     │
└─────────────────────────┴───────┘
```

`--by` slices the same figures by `status`, `kind`, `fiat`, `method`,
`instance`, `period` (calendar months), `hour` or `weekday` (histograms):

```console
$ bestiario stats orders --by fiat --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────┬────────┐
│ metric                      ┆ value  │
╞═════════════════════════════╪════════╡
│ orders.ARS.created          ┆ 2      │
│ orders.ARS.completed        ┆ 0      │
│ orders.ARS.canceled         ┆ 0      │
│ orders.ARS.completion_rate  ┆ —      │
│ orders.ARS.abandonment_rate ┆ 0.0%   │
│ orders.ARS.created_delta    ┆ —      │
│ orders.ARS.completed_delta  ┆ —      │
│ orders.ARS.open_now         ┆ 1      │
│ orders.ARS.in_progress_now  ┆ 1      │
│ orders.BRL.created          ┆ 2      │
│ orders.BRL.completed        ┆ 0      │
│ orders.BRL.canceled         ┆ 1      │
│ orders.BRL.completion_rate  ┆ 0.0%   │
│ orders.BRL.abandonment_rate ┆ 50.0%  │
│ orders.BRL.created_delta    ┆ —      │
│ orders.BRL.completed_delta  ┆ —      │
│ orders.BRL.open_now         ┆ 1      │
│ orders.BRL.in_progress_now  ┆ 0      │
│ orders.CUP.created          ┆ 1      │
│ orders.CUP.completed        ┆ 1      │
│ orders.CUP.canceled         ┆ 0      │
│ orders.CUP.completion_rate  ┆ 100.0% │
│ orders.CUP.abandonment_rate ┆ 0.0%   │
│ orders.CUP.created_delta    ┆ —      │
│ orders.CUP.completed_delta  ┆ —      │
│ orders.CUP.open_now         ┆ 0      │
│ orders.CUP.in_progress_now  ┆ 0      │
│ orders.EUR.created          ┆ 2      │
│ orders.EUR.completed        ┆ 0      │
│ orders.EUR.canceled         ┆ 0      │
│ orders.EUR.completion_rate  ┆ —      │
│ orders.EUR.abandonment_rate ┆ 0.0%   │
│ orders.EUR.created_delta    ┆ —      │
│ orders.EUR.completed_delta  ┆ —      │
│ orders.EUR.open_now         ┆ 2      │
│ orders.EUR.in_progress_now  ┆ 0      │
│ orders.USD.created          ┆ 1      │
│ orders.USD.completed        ┆ 0      │
│ orders.USD.canceled         ┆ 0      │
│ orders.USD.completion_rate  ┆ —      │
│ orders.USD.abandonment_rate ┆ 0.0%   │
│ orders.USD.created_delta    ┆ —      │
│ orders.USD.completed_delta  ┆ —      │
│ orders.USD.open_now         ┆ 0      │
│ orders.USD.in_progress_now  ┆ 0      │
└─────────────────────────────┴────────┘
```

A rate over nothing — no order completed or canceled, no previous month to
grow from — is reported as `—` in a table and `null` in JSON, never as
zero: zero is an answer, and this is the absence of one.

`--by day` cuts the window into UTC calendar days instead, one block per
day and a Δ against the day before:

```console
$ bestiario stats orders --by day --from 2026-08-25 --until 2026-08-28
2026-08-25T00:00:00+00:00 — 2026-08-28T00:00:00+00:00
┌────────────────────────────────────┬─────────┐
│ metric                             ┆ value   │
╞════════════════════════════════════╪═════════╡
│ orders.2026-08-25.created          ┆ —       │
│ orders.2026-08-25.completed        ┆ —       │
│ orders.2026-08-25.canceled         ┆ —       │
│ orders.2026-08-25.completion_rate  ┆ —       │
│ orders.2026-08-25.abandonment_rate ┆ —       │
│ orders.2026-08-25.created_delta    ┆ —       │
│ orders.2026-08-25.completed_delta  ┆ —       │
│ orders.2026-08-26.created          ┆ 8       │
│ orders.2026-08-26.completed        ┆ 1       │
│ orders.2026-08-26.canceled         ┆ 1       │
│ orders.2026-08-26.completion_rate  ┆ 50.0%   │
│ orders.2026-08-26.abandonment_rate ┆ 12.5%   │
│ orders.2026-08-26.created_delta    ┆ —       │
│ orders.2026-08-26.completed_delta  ┆ —       │
│ orders.2026-08-27.created          ┆ 0       │
│ orders.2026-08-27.completed        ┆ 0       │
│ orders.2026-08-27.canceled         ┆ 0       │
│ orders.2026-08-27.completion_rate  ┆ —       │
│ orders.2026-08-27.abandonment_rate ┆ —       │
│ orders.2026-08-27.created_delta    ┆ -100.0% │
│ orders.2026-08-27.completed_delta  ┆ -100.0% │
└────────────────────────────────────┴─────────┘
```

Every day of the window is there, quiet ones included, so a chart cannot
draw a line across a gap that was never published. A day the archive
predates is the one thing that is *not* zero: nobody indexed it, and saying
zero would claim the network published nothing when the truth is that
bestiario was not there. Those days keep their rows and report `—`, and a Δ
against one of them is `—` too. Relays hold orders for about a fortnight,
so any window reaching back past your first `backfill` will show them.

The same applies to a kind nothing ever asked for. `backfill --kind 38383`
indexes orders and leaves disputes untouched; a later `stats disputes --by
day` then has no dispute history to speak from, and reports `—` throughout
rather than a month of confident zeros. Run `backfill` without `--kind`, or
`sync`, and the days it covered become answerable — zeros included, because
a day nobody disputed anything is a fact. A report combining families needs
all of them: the dispute rate divides disputes by orders, so a day is
answerable only when both reach it.

### Volume

```console
$ bestiario stats volume --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌────────────────────────────┬────────────┐
│ metric                     ┆ value      │
╞════════════════════════════╪════════════╡
│ volume.sats                ┆ 1361 sats  │
│ volume.completed           ┆ 1          │
│ volume.ticket_avg          ┆ 1361 sats  │
│ volume.ticket_p50          ┆ 1361 sats  │
│ volume.ticket_p90          ┆ 1361 sats  │
│ volume.largest             ┆ 1361 sats  │
│ volume.size.lt_10k         ┆ 1          │
│ volume.size.10k_50k        ┆ 0          │
│ volume.size.50k_200k       ┆ 0          │
│ volume.size.200k_1m        ┆ 0          │
│ volume.size.gt_1m          ┆ 0          │
│ volume.buy_sats            ┆ 0 sats     │
│ volume.sell_sats           ┆ 1361 sats  │
│ volume.fiat.CUP.total      ┆ 800.00 CUP │
│ volume.fiat.CUP.orders     ┆ 1          │
│ volume.fiat.CUP.ticket_avg ┆ 800.00 CUP │
│ volume.fiat.CUP.ticket_p50 ┆ 800.00 CUP │
│ volume.fiat.CUP.ticket_p90 ┆ 800.00 CUP │
└────────────────────────────┴────────────┘
```

Sats and fiat traded by the orders that reached `success` in the window,
dated by the moment they did: total, average and p50/p90 ticket, the
largest order, the size buckets and the maker's side. Fiat figures are per
currency and cover fixed-amount orders only — a range order names no single
amount, so it has sats to add and no fiat to add. `--by` slices by `kind`,
`fiat`, `instance` or `period`, one block per slice that completed
something in the window — only completed orders are read, so the cost of
a week's report is a week's orders, not the history:

```console
$ bestiario stats volume --by kind --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────────┬────────────┐
│ metric                          ┆ value      │
╞═════════════════════════════════╪════════════╡
│ volume.sell.sats                ┆ 1361 sats  │
│ volume.sell.completed           ┆ 1          │
│ volume.sell.ticket_avg          ┆ 1361 sats  │
│ volume.sell.ticket_p50          ┆ 1361 sats  │
│ volume.sell.ticket_p90          ┆ 1361 sats  │
│ volume.sell.largest             ┆ 1361 sats  │
│ volume.sell.size.lt_10k         ┆ 1          │
│ volume.sell.size.10k_50k        ┆ 0          │
│ volume.sell.size.50k_200k       ┆ 0          │
│ volume.sell.size.200k_1m        ┆ 0          │
│ volume.sell.size.gt_1m          ┆ 0          │
│ volume.sell.buy_sats            ┆ 0 sats     │
│ volume.sell.sell_sats           ┆ 1361 sats  │
│ volume.sell.fiat.CUP.total      ┆ 800.00 CUP │
│ volume.sell.fiat.CUP.orders     ┆ 1          │
│ volume.sell.fiat.CUP.ticket_avg ┆ 800.00 CUP │
│ volume.sell.fiat.CUP.ticket_p50 ┆ 800.00 CUP │
│ volume.sell.fiat.CUP.ticket_p90 ┆ 800.00 CUP │
└─────────────────────────────────┴────────────┘
```

`--in <CURRENCY>` adds the conversion into a reference currency: each
completed order at the rate its instance had published by the moment it
settled, summed. The figure is *inferred* — the rate is one instance's
snapshot, minutes old — and every row of it says so, with what qualifies it
in the `error` column: the age of the oldest rate used, how many orders were
priced on another instance's snapshot, and how many had no rate at all and
are left out of the sum:

```console
$ bestiario stats volume --in USD --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌───────────────────────────────────┬────────────┬────────────────────────────────────────────────────────────────────────┐
│ metric                            ┆ value      ┆ error                                                                  │
╞═══════════════════════════════════╪════════════╪════════════════════════════════════════════════════════════════════════╡
│ volume.sats                       ┆ 1361 sats  ┆                                                                        │
│ volume.completed                  ┆ 1          ┆                                                                        │
│ volume.ticket_avg                 ┆ 1361 sats  ┆                                                                        │
│ volume.ticket_p50                 ┆ 1361 sats  ┆                                                                        │
│ volume.ticket_p90                 ┆ 1361 sats  ┆                                                                        │
│ volume.largest                    ┆ 1361 sats  ┆                                                                        │
│ volume.size.lt_10k                ┆ 1          ┆                                                                        │
│ volume.size.10k_50k               ┆ 0          ┆                                                                        │
│ volume.size.50k_200k              ┆ 0          ┆                                                                        │
│ volume.size.200k_1m               ┆ 0          ┆                                                                        │
│ volume.size.gt_1m                 ┆ 0          ┆                                                                        │
│ volume.buy_sats                   ┆ 0 sats     ┆                                                                        │
│ volume.sell_sats                  ┆ 1361 sats  ┆                                                                        │
│ volume.fiat.CUP.total             ┆ 800.00 CUP ┆                                                                        │
│ volume.fiat.CUP.orders            ┆ 1          ┆                                                                        │
│ volume.fiat.CUP.ticket_avg        ┆ 800.00 CUP ┆                                                                        │
│ volume.fiat.CUP.ticket_p50        ┆ 800.00 CUP ┆                                                                        │
│ volume.fiat.CUP.ticket_p90        ┆ 800.00 CUP ┆                                                                        │
│ volume.in.USD.total (inf)         ┆ —          ┆ no rate used; 1 with no usable rate within 300s (1361 sats excluded)   │
│ volume.in.USD.orders (inf)        ┆ 0          ┆ orders with a rate published at or before success_at                   │
│ volume.in.USD.unpriced_sats (inf) ┆ 1361 sats  ┆ sats of the orders with no usable rate at success_at; not in the total │
│ volume.in.USD.rate_age_max (inf)  ┆ —          ┆ age of the oldest snapshot used                                        │
└───────────────────────────────────┴────────────┴────────────────────────────────────────────────────────────────────────┘
```

Here the one completed order settled hours before the first rate snapshot
was captured, so there is nothing to price it with: the total is `—`, not
`0.00 USD`, and the excluded sats are counted.

### Timing

```console
$ bestiario stats timing --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌──────────────────────────────────────┬───────┐
│ metric                               ┆ value │
╞══════════════════════════════════════╪═══════╡
│ timing.time_to_fill_samples          ┆ 0     │
│ timing.time_to_fill_p50              ┆ —     │
│ timing.time_to_fill_p90              ┆ —     │
│ timing.time_to_complete_samples      ┆ 0     │
│ timing.time_to_complete_p50          ┆ —     │
│ timing.time_to_complete_p90          ┆ —     │
│ timing.full_cycle_samples            ┆ 0     │
│ timing.full_cycle_p50                ┆ —     │
│ timing.full_cycle_p90                ┆ —     │
│ timing.time_to_cancel_samples        ┆ 0     │
│ timing.time_to_cancel_p50            ┆ —     │
│ timing.time_to_cancel_p90            ┆ —     │
│ timing.book_size                     ┆ 4     │
│ timing.book_age_avg                  ┆ 16.9h │
│ timing.funnel.created                ┆ 5     │
│ timing.funnel.taken                  ┆ 0     │
│ timing.funnel.taken_share            ┆ 0.0%  │
│ timing.funnel.completed              ┆ 0     │
│ timing.funnel.canceled_taken         ┆ 0     │
│ timing.funnel.canceled_untaken       ┆ 0     │
│ timing.funnel.canceled_untaken_share ┆ 0.0%  │
│ timing.funnel.expired_untaken        ┆ 1     │
│ timing.funnel.open                   ┆ 4     │
│ timing.unknown_origin                ┆ 3     │
│ timing.regressed                     ┆ 0     │
└──────────────────────────────────────┴───────┘
```

Every duration is the gap between two *observed* versions of the same
order: time to fill (`in-progress − pending`), time to complete
(`success − in-progress`), the full cycle (`success − pending`) and time
to cancel (`canceled − pending`), as nearest-rank p50/p90 over the orders
whose gap *ended* in the window, each with the number of samples it is
taken over — the populations differ, since each gap needs its own two
versions. `book_size` and `book_age_avg` are about now: the `pending`
orders seen from their book entry, not taken, not ended, not expired.

The funnel is over the orders whose `pending` version was seen in the
window: how many found a taker (an `in-progress` version, or a success,
which the protocol cannot reach without one), how many completed, how many
were canceled after a taker or with none seen, how many sat past their
expiry with no ending seen (`expired_untaken`), and how many are still
open. `unknown_origin` counts the orders first seen at a later stage —
usual in a backfill, since a relay keeps only an order's latest version —
which belong to no cohort and anchor no duration; `regressed` counts
orders carrying both a success and a cancellation, of which only the
earlier is counted. Slices — `--by fiat`, `--by method`, `--by kind`,
`--by instance` — go by the order's first version seen, so a later
republication cannot move an entry to another slice.

In this corpus every order was captured in a single version: the five seen
at `pending` form the cohort, the three seen already in progress or ended
are of unknown origin, and no duration can be measured.

### Market

```console
$ bestiario stats market --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌───────────────────────────────┬──────────────────────────────────────────────────────────────────────────────┐
│ metric                        ┆ value                                                                        │
╞═══════════════════════════════╪══════════════════════════════════════════════════════════════════════════════╡
│ market.orders                 ┆ 8                                                                            │
│ market.buy_orders_share       ┆ 50.0%                                                                        │
│ market.buy_volume_share       ┆ 0.0%                                                                         │
│ market.premium_avg            ┆ 10.0%                                                                        │
│ market.premium_p50            ┆ 10.0%                                                                        │
│ market.premium_p50_buy        ┆ —                                                                            │
│ market.premium_p50_sell       ┆ 10.0%                                                                        │
│ market.premium_spread         ┆ —                                                                            │
│ market.market_price_share     ┆ 50.0%                                                                        │
│ market.range_share            ┆ 50.0%                                                                        │
│ market.range_width_avg        ┆ 82.8%                                                                        │
│ market.fiat_top3_by_orders    ┆ ARS 2, BRL 2, EUR 2                                                          │
│ market.fiat_top3_orders_share ┆ 75.0%                                                                        │
│ market.fiat_hhi_orders        ┆ 21.9%                                                                        │
│ market.fiat_top3_by_volume    ┆ CUP 1361 sats                                                                │
│ market.fiat_top3_volume_share ┆ 100.0%                                                                       │
│ market.fiat_hhi_volume        ┆ 100.0%                                                                       │
│ market.method_top3_by_orders  ┆ CBU 2, CVU 2, BBVA Efectivo Móvil 1                                          │
│ market.method_top3_by_volume  ┆ Efectivo 1361 sats, EnZona 1361 sats                                         │
│ market.new_fiats              ┆ ARS, BRL, CUP, EUR, USD                                                      │
│ market.new_methods            ┆ BBVA Efectivo Móvil, Belo, CBU, CVU, DePix, Efectivo, EnZona, Lemon, +7 more │
└───────────────────────────────┴──────────────────────────────────────────────────────────────────────────────┘
```

Which way the book leans and at what price. Pressure is the buy share of
the orders created in the window and of the sats completed in it; the
premiums are those of the completed orders — an open order's premium is
only an ask — as average, median, median by side and the spread between
the sides. `market_price_share` is the share of orders born with `amt = 0`,
priced when taken; `range_share` the share published as `[min, max]`, with
the mean *relative* width `(max − min) ÷ max` — a block that holds ARS
beside EUR cannot average their widths, and the relative form compares.
Currencies and payment methods are ranked by orders created and by sats
completed, and `new_fiats` / `new_methods` list what was seen for the
first time in the window — the first eight, and a count of the rest; `—`
on those two rows means none were, not that none could be counted. The
methods counted are those of an order's first version, what the maker put
on the book: a `pm` amended days later was not on offer at creation, and
dated by that order it would hide a genuine first sighting.

An order names one currency and may name several payment methods, and it
is credited in full to each method it names — above, one completed order
of 1 361 sats offered over two methods shows 1 361 against each, so the
method rows add up to more than the volume traded. The sats are
attributed to a method, not divided between them; dividing them would
invent a figure nobody published. That is why only the currencies carry
the concentration rows: the top-three share, and the
Herfindahl–Hirschman index — an index between `1/n` and `1` rather than a
share of anything, shown as a percentage for the column it sits in.

`--by fiat`, `--by kind` and `--by instance` slice it. A fiat slice drops
the currency ranking, which says nothing about one currency, and gains
`range_width_fiat_avg`: the mean `max − min` in that currency, which only
a single-currency block can state — relative width alone would call
`[10, 100]` wider than `[900, 1000]`. Every row of a slice is that
slice's own, the first sightings included: `market.buy.new_methods` names
the methods whose first *buy* order fell in the window, so a method a
seller has offered for a year is new to the `buy` block the day a buyer
first names it, and `market.<instance>.new_fiats` names what is new to
that instance.

### Exchange rates

```console
$ bestiario stats rates --fiat USD --instance Mostro
2026-07-28T03:06:40+00:00 — 2026-08-27T03:06:40+00:00
┌────────────────────────────────────┬──────────────┐
│ metric                             ┆ value        │
╞════════════════════════════════════╪══════════════╡
│ rates.feeds                        ┆ 1            │
│ rates.fresh                        ┆ 0            │
│ rates.stale                        ┆ 0            │
│ rates.dead                         ┆ 1            │
│ rates.silent                       ┆ 0            │
│ rates.skewed                       ┆ 0            │
│ rates.currencies                   ┆ 141          │
│ rates.USD.quoted_by                ┆ 1            │
│ rates.USD.comparable               ┆ 0            │
│ rates.USD.low                      ┆ —            │
│ rates.USD.high                     ┆ —            │
│ rates.USD.disparity                ┆ —            │
│ rates.USD.Mostro (6320ee5e)        ┆ 78614.25 USD │
│ rates.Mostro (6320ee5e).age        ┆ 16.5h        │
│ rates.Mostro (6320ee5e).status     ┆ dead         │
│ rates.Mostro (6320ee5e).currencies ┆ 141          │
└────────────────────────────────────┴──────────────┘
```

What each instance quotes right now, and how alive its feed is. These are
the only figures in the tool that are not taken over the window: a feed is
a live thing, and §6.8 asks what it says *now* — the window still heads the
report, as everywhere.

A feed is `fresh` while its latest snapshot is under five minutes old, the
bound a rate has to price a trade; `stale` past that but within the ten
minutes a kind 30078 event declares itself valid for through its NIP-40
`expiration`; `dead` past that expiry, with nothing since; `silent` when
the instance has published no rate at all; and `skewed` when its latest
snapshot is dated in the future, which is a clock nobody shares rather than
an age. Every instance falls in exactly one of the five, and the counts add
up to the statuses listed below them. Rate snapshots carry no `y` tag, so
bestiario stores one only from a pubkey it has already seen publishing as
an instance — an unvouched feed is not a feed, and a stored snapshot whose
publisher is missing from the bestiary fails the report rather than being
quietly readmitted to it.

`--fiat <CURRENCY>` adds the currency's block: who quotes it, the cheapest
and dearest quote, and the disparity between them. The disparity is about
*now*: only the quotes that are still fresh set `low`, `high` and the
ratio, because two prices an hour apart differ by the market moving — which
is not a disagreement between instances — and a price whose own event has
expired is not what the feed quotes today. Everyone quoting the currency is
still counted under `quoted_by`, so a currency nobody quotes live says `—`
rather than reporting the disagreement of two dead snapshots; one
comparable quote disagrees with nobody, and that row says `—` too. Without
`--instance` the report covers every instance in the bestiary, one block
each.

### Series

```console
$ bestiario series orders.created --by day --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────────┬───────┐
│ metric                          ┆ value │
╞═════════════════════════════════╪═══════╡
│ orders.created.2026-08-23       ┆ 0     │
│ orders.created.2026-08-23.delta ┆ —     │
│ orders.created.2026-08-24       ┆ 0     │
│ orders.created.2026-08-24.delta ┆ —     │
│ orders.created.2026-08-25       ┆ 0     │
│ orders.created.2026-08-25.delta ┆ —     │
│ orders.created.2026-08-26       ┆ 8     │
│ orders.created.2026-08-26.delta ┆ —     │
└─────────────────────────────────┴───────┘
```

Any metric of the families above, once per bucket, with the change against
the bucket before it. `--by` takes `day`, `week`, `month` or `year`;
`--split instance|kind|fiat` plots one line per slice:

```console
$ bestiario series volume.sats --by day --split kind --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌───────────────────────────────────┬───────────┐
│ metric                            ┆ value     │
╞═══════════════════════════════════╪═══════════╡
│ volume.sats.buy.2026-08-23        ┆ 0 sats    │
│ volume.sats.buy.2026-08-23.delta  ┆ —         │
│ volume.sats.buy.2026-08-24        ┆ 0 sats    │
│ volume.sats.buy.2026-08-24.delta  ┆ —         │
│ volume.sats.buy.2026-08-25        ┆ 0 sats    │
│ volume.sats.buy.2026-08-25.delta  ┆ —         │
│ volume.sats.buy.2026-08-26        ┆ 0 sats    │
│ volume.sats.buy.2026-08-26.delta  ┆ —         │
│ volume.sats.sell.2026-08-23       ┆ 0 sats    │
│ volume.sats.sell.2026-08-23.delta ┆ —         │
│ volume.sats.sell.2026-08-24       ┆ 0 sats    │
│ volume.sats.sell.2026-08-24.delta ┆ —         │
│ volume.sats.sell.2026-08-25       ┆ 0 sats    │
│ volume.sats.sell.2026-08-25.delta ┆ —         │
│ volume.sats.sell.2026-08-26       ┆ 1361 sats │
│ volume.sats.sell.2026-08-26.delta ┆ —         │
└───────────────────────────────────┴───────────┘
```

The Δ is a relative change for a magnitude — a count, a sum, a duration —
since that is what "grew by a third" means, and the arithmetic difference
for a figure that is already a proportion: a completion rate going from 20%
to 30% rose by ten points, not by half. The first bucket has nothing to
have changed from, and a change from zero is not a proportion of anything;
both read `—` — which, over a corpus this sparse, is every bucket of the
examples above.

The metric name is whatever the reports call it, and nothing keeps a
separate list: a metric a family gains is one `series` can plot the day it
lands. That includes the names the data gives rather than the code —
`volume.fiat.ARS.total` exists because an ARS order completed — so which
names are plottable depends on the window, and a name that does not exist
in it is answered with the ones that do. A converted figure names its own
currency and is priced from the snapshots exactly as `stats volume --in
USD` is: `series volume.in.USD.total` needs no flag of its own.

An inferred figure stays inferred once it is a bucket: `(inf)` in the
table, `"kind": "inferred"` in the JSON, with the qualification the report
gives it — and so does a Δ between two of them, since a change between two
estimates is an estimate.

Two kinds of figure are refused rather than plotted: those about *now*
(`orders.open_now`), which would be the same number in every bucket, and
those that are already a change against a previous period
(`orders.created_delta`), since a Δ of a Δ answers nothing. So is a range
with more buckets than a table anybody reads — before the buckets are
built, not after.

### Dev fees

```console
$ bestiario stats dev-fees --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌────────────────────────────────────┬──────────┬──────────────────────────────────────────────┐
│ metric                             ┆ value    ┆ error                                        │
╞════════════════════════════════════╪══════════╪══════════════════════════════════════════════╡
│ dev_fees.total_sats                ┆ 117 sats ┆                                              │
│ dev_fees.paid                      ┆ 2        ┆                                              │
│ dev_fees.coverage                  ┆ —        ┆                                              │
│ dev_fees.latency_p50               ┆ —        ┆                                              │
│ dev_fees.latency_p90               ┆ —        ┆                                              │
│ dev_fees.duplicates                ┆ 0        ┆                                              │
│ dev_fees.orphans                   ┆ 2        ┆                                              │
│ dev_fees.implied_volume (inf)      ┆ —        ┆ no fee inverted; 2 fees with no fee in force │
│ dev_fees.with_fee_volume           ┆ —        ┆                                              │
│ dev_fees.implied_vs_observed (inf) ┆ —        ┆ no fee names a settled order                 │
└────────────────────────────────────┴──────────┴──────────────────────────────────────────────┘
```

An order paid for twice — a known daemon bug — counts once towards the
total and once under `duplicates`. `coverage` is the share of completed
orders that produced a fee, over the instances known to charge one.
`--by instance` and `--by period` slice it.

The last three rows are the comparison of SPEC §6.6. A dev fee is
`round(fee × amount × pct)`, so each fee *implies* an order of about
`fee ÷ (fee_in_force × pct)` sats — including the fees whose order has
already expired off the relays, which is what makes the figure worth
having. It is inferred twice over, and the `error` column says how: one
sat of rounding on a fee is `1 ÷ (fee × pct)` sats of volume, and `pct` —
the share of its fee an instance forwards — is not published by anyone, so
it is the `[assumptions]` of `settings.toml` (`0.30`, the daemon default,
unless overridden per instance).

A third qualification outweighs both in a backfill: a fee whose instance
never published a `fee` the projection still has cannot be inverted at
all. Those fees are not in the sum and no rounding bound covers them, so
whenever there is one the `error` column says `lower bound: n of m fees
inverted` and the figure is to be read as the floor it is.

`with_fee_volume` is the observed side: `∑ amount_sats` of the orders that
a fee names **and** that reached `success` — a fee paid against an order
the relays no longer have, or one that was later canceled, has no observed
volume to show, and the row is `—` rather than `0` when none of them does.
It is observed in the strict sense: it does not move with what could be
inverted. `implied_vs_observed` is `implied ÷ observed − 1` over the
intersection — the fees that were inverted *and* name a settled order of a
positive amount — so a positive figure means the instance forwards more
than assumed. Orders still known at `amt = 0` (market price, never
amended: SPEC §3) are out of the ratio, which would otherwise divide by
nothing, and the column says how many.

The two sides are dated differently on purpose: a fee counts in the window
its own event falls in, while the order it names may have settled in the
previous one. `with_fee_volume` for a month is therefore not the same set
as `volume.total` for that month, and the two are not meant to reconcile.

Here every fee names an order the relays no longer have and the instance's
fee at the time is unknown, so nothing can be inverted and the rows say
so.

### Disputes

```console
$ bestiario stats disputes --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌──────────────────────────────────┬──────────────────────────────────────┐
│ metric                           ┆ value                                │
╞══════════════════════════════════╪══════════════════════════════════════╡
│ disputes.opened                  ┆ 5                                    │
│ disputes.status.initiated        ┆ 1                                    │
│ disputes.status.in_progress      ┆ 1                                    │
│ disputes.status.seller_refunded  ┆ 2                                    │
│ disputes.status.settled          ┆ 1                                    │
│ disputes.status.released         ┆ 0                                    │
│ disputes.initiator.buyer         ┆ 60.0%                                │
│ disputes.initiator.seller        ┆ 40.0%                                │
│ disputes.rate                    ┆ 250.0%                               │
│ disputes.resolved                ┆ 3                                    │
│ disputes.outcome.seller_refunded ┆ 66.7%                                │
│ disputes.outcome.settled         ┆ 33.3%                                │
│ disputes.outcome.released        ┆ 0.0%                                 │
│ disputes.resolution_p50          ┆ 1.4h                                 │
│ disputes.resolution_p90          ┆ 2.0h                                 │
│ disputes.open_now                ┆ 2                                    │
│ disputes.open.1.id               ┆ c6ebce7e-e521-4df3-a8c5-24301145eb66 │
│ disputes.open.1.age              ┆ 3.1d                                 │
│ disputes.open.2.id               ┆ c402fff7-5255-4894-8105-dfb98a5981d0 │
│ disputes.open.2.age              ┆ 2.1d                                 │
└──────────────────────────────────┴──────────────────────────────────────┘
```

`--by status` and `--by initiator` print the histograms alone; `--by
instance` and `--by period` one block per slice.

### The bestiary

```console
$ bestiario instances --instance "Mostro Brasil" --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────────────────────────────┬──────────────────────────────────────────────────────────────────┐
│ metric                                              ┆ value                                                            │
╞═════════════════════════════════════════════════════╪══════════════════════════════════════════════════════════════════╡
│ instances.Mostro Brasil (ec3c3e00).pubkey           ┆ ec3c3e00a04aa9e0c040fcef2dc7767d66c6c93d5dd2b39ba937b820ddf23610 │
│ instances.Mostro Brasil (ec3c3e00).name             ┆ Mostro Brasil                                                    │
│ instances.Mostro Brasil (ec3c3e00).mostro_version   ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).protocol_version ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).fee              ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).min_order        ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).max_order        ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).fiat             ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).ln_networks      ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).bond             ┆ —                                                                │
│ instances.Mostro Brasil (ec3c3e00).first_seen       ┆ 2026-08-26T10:03:46+00:00                                        │
│ instances.Mostro Brasil (ec3c3e00).last_seen        ┆ 2026-08-26T10:36:53+00:00                                        │
│ instances.Mostro Brasil (ec3c3e00).silent_for       ┆ 16.5h                                                            │
│ instances.Mostro Brasil (ec3c3e00).silent           ┆ no                                                               │
│ instances.Mostro Brasil (ec3c3e00).created          ┆ 2                                                                │
└─────────────────────────────────────────────────────┴──────────────────────────────────────────────────────────────────┘
```

Without `--instance`, one block per known instance. An instance is *silent*
after a week without any event. What an instance never published — a third
of the network publishes no name — is `—`, not blank.

One instance, with its own figures and its share of the network:

```console
$ bestiario instance Mostro --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌──────────────────────────────────┬──────────────────────────────────────────────────────────────────┐
│ metric                           ┆ value                                                            │
╞══════════════════════════════════╪══════════════════════════════════════════════════════════════════╡
│ instance.pubkey                  ┆ 6320ee5edbaeb9a00d7c4768e472e277539aa993007c43d75ce00a38dff4d425 │
│ instance.name                    ┆ Mostro                                                           │
│ instance.mostro_version          ┆ —                                                                │
│ instance.protocol_version        ┆ —                                                                │
│ instance.fee                     ┆ —                                                                │
│ instance.min_order               ┆ —                                                                │
│ instance.max_order               ┆ —                                                                │
│ instance.fiat                    ┆ —                                                                │
│ instance.ln_networks             ┆ —                                                                │
│ instance.bond                    ┆ —                                                                │
│ instance.first_seen              ┆ 2026-08-25T01:24:38+00:00                                        │
│ instance.last_seen               ┆ 2026-08-26T10:39:33+00:00                                        │
│ instance.silent_for              ┆ 16.5h                                                            │
│ instance.silent                  ┆ no                                                               │
│ orders.created                   ┆ 4                                                                │
│ orders.completed                 ┆ 0                                                                │
│ orders.canceled                  ┆ 0                                                                │
│ orders.completion_rate           ┆ —                                                                │
│ orders.abandonment_rate          ┆ 0.0%                                                             │
│ orders.created_delta             ┆ —                                                                │
│ orders.completed_delta           ┆ —                                                                │
│ orders.open_now                  ┆ 2                                                                │
│ orders.in_progress_now           ┆ 1                                                                │
│ volume.sats                      ┆ 0 sats                                                           │
│ dev_fees.total_sats              ┆ 116 sats                                                         │
│ dev_fees.paid                    ┆ 1                                                                │
│ dev_fees.coverage                ┆ —                                                                │
│ dev_fees.latency_p50             ┆ —                                                                │
│ dev_fees.latency_p90             ┆ —                                                                │
│ dev_fees.duplicates              ┆ 0                                                                │
│ dev_fees.orphans                 ┆ 1                                                                │
│ disputes.opened                  ┆ 1                                                                │
│ disputes.status.initiated        ┆ 0                                                                │
│ disputes.status.in_progress      ┆ 1                                                                │
│ disputes.status.seller_refunded  ┆ 0                                                                │
│ disputes.status.settled          ┆ 0                                                                │
│ disputes.status.released         ┆ 0                                                                │
│ disputes.initiator.buyer         ┆ 100.0%                                                           │
│ disputes.initiator.seller        ┆ 0.0%                                                             │
│ disputes.rate                    ┆ 100.0%                                                           │
│ disputes.resolved                ┆ 0                                                                │
│ disputes.outcome.seller_refunded ┆ —                                                                │
│ disputes.outcome.settled         ┆ —                                                                │
│ disputes.outcome.released        ┆ —                                                                │
│ disputes.resolution_p50          ┆ —                                                                │
│ disputes.resolution_p90          ┆ —                                                                │
│ disputes.open_now                ┆ 1                                                                │
│ disputes.open.1.id               ┆ c402fff7-5255-4894-8105-dfb98a5981d0                             │
│ disputes.open.1.age              ┆ 2.1d                                                             │
│ share.orders                     ┆ 50.0%                                                            │
│ share.volume                     ┆ 0.0%                                                             │
└──────────────────────────────────┴──────────────────────────────────────────────────────────────────┘
```

### One currency's market

```console
$ bestiario market ARS --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────────────┬───────────────────────────────────────────┐
│ metric                              ┆ value                                     │
╞═════════════════════════════════════╪═══════════════════════════════════════════╡
│ market.ARS.orders                   ┆ 2                                         │
│ market.ARS.buy_orders_share         ┆ 50.0%                                     │
│ market.ARS.buy_volume_share         ┆ —                                         │
│ market.ARS.premium_avg              ┆ —                                         │
│ market.ARS.premium_p50              ┆ —                                         │
│ market.ARS.premium_p50_buy          ┆ —                                         │
│ market.ARS.premium_p50_sell         ┆ —                                         │
│ market.ARS.premium_spread           ┆ —                                         │
│ market.ARS.market_price_share       ┆ 50.0%                                     │
│ market.ARS.range_share              ┆ 50.0%                                     │
│ market.ARS.range_width_avg          ┆ 66.7%                                     │
│ market.ARS.range_width_fiat_avg     ┆ 50000.00 ARS                              │
│ market.ARS.method_top3_by_orders    ┆ CBU 2, CVU 2, Belo 1                      │
│ market.ARS.method_top3_by_volume    ┆ —                                         │
│ market.ARS.new_methods              ┆ Belo, CBU, CVU, Lemon, MODO, Mercado Pago │
│ market.ARS.time_to_fill_samples     ┆ 0                                         │
│ market.ARS.time_to_fill_p50         ┆ —                                         │
│ market.ARS.time_to_fill_p90         ┆ —                                         │
│ market.ARS.book_size                ┆ 1                                         │
│ market.ARS.instances                ┆ 1                                         │
│ market.ARS.instances_top3_by_orders ┆ Mostro (6320ee5e) 2                       │
│ market.ARS.instances_top3_by_volume ┆ —                                         │
└─────────────────────────────────────┴───────────────────────────────────────────┘
```

Everything the reports know about one currency, in one place: which way its
book leans and at what premium, how it is priced, which payment methods it
is offered over, how long an order takes to find a taker, and which
instances trade it at all. The figures are the ones `stats market` and
`stats timing` report, each over the cohort its own family uses, so nothing
here can drift from where it is quoted from: the structure rows count the
orders *standing* in the currency — a currency is what an order's latest
version says it is — and the timing rows count the orders that *entered the
book* in it, because a time-to-fill is measured from the book entry and an
order amended from ARS to USD waited in ARS. The two differ only for an
order amended into another currency. Ranking currencies inside a single
currency would say nothing, so those rows are absent.

### Comparison

```console
$ bestiario compare --network regtest --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌───────────────────────────┬───────────┬─────────────┬─────────────────┬─────┬───────────────┬──────────────┬─────────┐
│ instance                  ┆ completed ┆ volume_sats ┆ completion_rate ┆ fee ┆ dev_fees_sats ┆ dispute_rate ┆ version │
╞═══════════════════════════╪═══════════╪═════════════╪═════════════════╪═════╪═══════════════╪══════════════╪═════════╡
│ Fostro testing (8c6a4452) ┆ 1         ┆ 1361 sats   ┆ 100.0%          ┆ —   ┆ 0 sats        ┆ —            ┆ —       │
└───────────────────────────┴───────────┴─────────────┴─────────────────┴─────┴───────────────┴──────────────┴─────────┘
```

One row per instance: completed orders, sats volume, completion rate, fee,
dev fees sent, dispute rate, version. Under `--network` only the instances
that traded on that network are compared — here one — and the dispute rate
is `—`, since disputes cannot be narrowed to a network.

### One order

```console
$ bestiario orders aac4b221-bcd0-47e2-bea8-b0251fc5bb89
2026-08-26T10:39:05+00:00 — 2026-08-26T10:39:06+00:00
┌────────────────────┬──────────────────────────────────────┐
│ metric             ┆ value                                │
╞════════════════════╪══════════════════════════════════════╡
│ order.id           ┆ aac4b221-bcd0-47e2-bea8-b0251fc5bb89 │
│ order.versions     ┆ 1                                    │
│ order.1.at         ┆ 2026-08-26T10:39:05+00:00            │
│ order.1.status     ┆ in-progress                          │
│ order.1.kind       ┆ buy                                  │
│ order.1.amount     ┆ 101446 sats                          │
│ order.1.fiat       ┆ 75000.00 ARS                         │
│ order.1.premium    ┆ 0.0%                                 │
│ order.1.expires_at ┆ 2026-08-26T23:09:21+00:00            │
└────────────────────┴──────────────────────────────────────┘
```

Every version the instance published, oldest first, then every dev fee that
names the order. This is the one report that shows events rather than
counts.

### JSON

```console
$ bestiario summary --json --from 2026-08-23 --until 2026-08-27
{
  "generated_at": "2026-08-27T03:06:40+00:00",
  "range": {
    "from": "2026-08-23T00:00:00+00:00",
    "until": "2026-08-27T00:00:00+00:00"
  },
  "metrics": [
    {
      "name": "summary.created",
      "kind": "observed",
      "unit": "count",
      "value": 8
    },
    {
      "name": "summary.completed",
      "kind": "observed",
      "unit": "count",
      "value": 1
    },
    {
      "name": "summary.completion_rate",
      "kind": "observed",
      "unit": "ratio",
      "value": 0.5
    },
    {
      "name": "summary.volume_sats",
      "kind": "observed",
      "unit": "sats",
      "value": 1361
    },
    {
      "name": "summary.active_instances",
      "kind": "observed",
      "unit": "count",
      "value": 4
    },
    {
      "name": "summary.top_fiat",
      "kind": "observed",
      "unit": "text",
      "value": "ARS (2), BRL (2), EUR (2)"
    },
    {
      "name": "summary.top_methods",
      "kind": "observed",
      "unit": "text",
      "value": "CBU (2), CVU (2), BBVA Efectivo Móvil (1)"
    },
    {
      "name": "summary.open_disputes",
      "kind": "observed",
      "unit": "count",
      "value": 2
    }
  ]
}
```

The envelope is the same for every report: `generated_at`, `range`, and
`metrics`, one flat record per figure with its `name`, its `kind`
(`observed` or `inferred`), its `unit` and its `value` — `null` for a
figure with nothing to report. Names are dotted paths, with the slice as a
segment (`orders.ARS.created`), so a consumer can split on the dot.

### Rebuilding

```console
$ bestiario rebuild
rebuild: 21 events replayed (0 unreadable), 8 orders and 5 disputes projected
```

Every table except the raw event archive is derived from it. `rebuild`
recomputes them; `rebuild --from-raw` replays the archive from scratch.
Neither talks to a relay.

## Development

```sh
cargo test --workspace            # unit, integration and the end-to-end suite
cargo clippy --workspace --all-targets -- -D warnings
```

`docs/SPEC.md` is the source of truth for event formats, the data model,
the metrics and the CLI; `docs/ROADMAP.md` is the plan, one row per pull
request. `docs/NOSTR-PUBLICATION.md` specifies one planned feature in full —
publishing the figures as signed Nostr events, so a reader trusts a pubkey
rather than a host — and is scheduled as phase 7 of the roadmap. The
aggregation layer is a separate crate, `crates/stats`, with no I/O
dependencies — a compile error, not a convention — so that the same figures
can be served over HTTP, or signed onto a relay, without touching them.
