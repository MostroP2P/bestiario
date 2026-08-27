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
| `[indexer].instances` | Pubkeys to follow, in hex. |
| `[indexer].accept_unknown_instances` | `true` to index every pubkey that publishes Mostro events, whether listed or not. Events from other platforms on the same relays (they exist) are turned away either way. |
| `[indexer].networks` | Which networks count: `mainnet` alone by default. |
| `[indexer].backfill_from` | How far back the first `backfill` reaches, as a unix timestamp. |
| `[database].url` | Where the archive lives: `sqlite://bestiario.db`. |

`bestiario` reads `settings.toml` from the current directory; `--config`
points it elsewhere. Every value is validated at startup, and a bad one is
an error naming the key rather than a report with a hole in it.

## First backfill

```console
$ bestiario backfill
backfill: 22 stored, 5 already known, 3 rejected
```

`backfill` walks each relay's history backwards, from now down to
`backfill_from`, one kind at a time, and stores what it finds. It prints a
summary of what was stored, what it had already seen and what it turned
away, and it is safe to run again: nothing is written twice. Relays keep
dev fees for about a year and orders for about a fortnight, so on a fresh
archive expect fees for orders you will never see; the reports call those
*orphans*.

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
  The default is the last thirty days.
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
`fiat`, `instance` or `period`:

```console
$ bestiario stats volume --by kind --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌─────────────────────────────────┬────────────┐
│ metric                          ┆ value      │
╞═════════════════════════════════╪════════════╡
│ volume.buy.sats                 ┆ 0 sats     │
│ volume.buy.completed            ┆ 0          │
│ volume.buy.ticket_avg           ┆ —          │
│ volume.buy.ticket_p50           ┆ —          │
│ volume.buy.ticket_p90           ┆ —          │
│ volume.buy.largest              ┆ —          │
│ volume.buy.size.lt_10k          ┆ 0          │
│ volume.buy.size.10k_50k         ┆ 0          │
│ volume.buy.size.50k_200k        ┆ 0          │
│ volume.buy.size.200k_1m         ┆ 0          │
│ volume.buy.size.gt_1m           ┆ 0          │
│ volume.buy.buy_sats             ┆ 0 sats     │
│ volume.buy.sell_sats            ┆ 0 sats     │
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
┌───────────────────────────────────┬────────────┬───────────────────────────────────────────────────────────────────────────────┐
│ metric                            ┆ value      ┆ error                                                                         │
╞═══════════════════════════════════╪════════════╪═══════════════════════════════════════════════════════════════════════════════╡
│ volume.sats                       ┆ 1361 sats  ┆                                                                               │
│ volume.completed                  ┆ 1          ┆                                                                               │
│ volume.ticket_avg                 ┆ 1361 sats  ┆                                                                               │
│ volume.ticket_p50                 ┆ 1361 sats  ┆                                                                               │
│ volume.ticket_p90                 ┆ 1361 sats  ┆                                                                               │
│ volume.largest                    ┆ 1361 sats  ┆                                                                               │
│ volume.size.lt_10k                ┆ 1          ┆                                                                               │
│ volume.size.10k_50k               ┆ 0          ┆                                                                               │
│ volume.size.50k_200k              ┆ 0          ┆                                                                               │
│ volume.size.200k_1m               ┆ 0          ┆                                                                               │
│ volume.size.gt_1m                 ┆ 0          ┆                                                                               │
│ volume.buy_sats                   ┆ 0 sats     ┆                                                                               │
│ volume.sell_sats                  ┆ 1361 sats  ┆                                                                               │
│ volume.fiat.CUP.total             ┆ 800.00 CUP ┆                                                                               │
│ volume.fiat.CUP.orders            ┆ 1          ┆                                                                               │
│ volume.fiat.CUP.ticket_avg        ┆ 800.00 CUP ┆                                                                               │
│ volume.fiat.CUP.ticket_p50        ┆ 800.00 CUP ┆                                                                               │
│ volume.fiat.CUP.ticket_p90        ┆ 800.00 CUP ┆                                                                               │
│ volume.in.USD.total (inf)         ┆ —          ┆ no rate used; 1 unpriced (1361 sats excluded)                                 │
│ volume.in.USD.orders (inf)        ┆ 0          ┆ orders with a rate published at or before success_at                          │
│ volume.in.USD.unpriced_sats (inf) ┆ 1361 sats  ┆ sats of the orders no instance had a rate for by success_at; not in the total │
│ volume.in.USD.rate_age_max (inf)  ┆ —          ┆ age of the oldest snapshot used                                               │
└───────────────────────────────────┴────────────┴───────────────────────────────────────────────────────────────────────────────┘
```

Here the one completed order settled hours before the first rate snapshot
was captured, so there is nothing to price it with: the total is `—`, not
`0.00 USD`, and the excluded sats are counted.

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
the mean relative width `(max − min) ÷ max`. Currencies and payment methods
are ranked by orders created and by sats completed, with the top-three
share and the Herfindahl–Hirschman index of each ranking, and `new_fiats`
/ `new_methods` list what was seen for the first time in the window — the
first eight, and a count of the rest. `--by fiat`, `--by kind` and
`--by instance` slice it; a fiat slice drops the currency ranking, which
says nothing about one currency.

### Dev fees

```console
$ bestiario stats dev-fees --from 2026-08-23 --until 2026-08-27
2026-08-23T00:00:00+00:00 — 2026-08-27T00:00:00+00:00
┌────────────────────────────────────┬──────────┬──────────────────────────────────────────────────────────┐
│ metric                             ┆ value    ┆ error                                                    │
╞════════════════════════════════════╪══════════╪══════════════════════════════════════════════════════════╡
│ dev_fees.total_sats                ┆ 117 sats ┆                                                          │
│ dev_fees.paid                      ┆ 2        ┆                                                          │
│ dev_fees.coverage                  ┆ —        ┆                                                          │
│ dev_fees.latency_p50               ┆ —        ┆                                                          │
│ dev_fees.latency_p90               ┆ —        ┆                                                          │
│ dev_fees.duplicates                ┆ 0        ┆                                                          │
│ dev_fees.orphans                   ┆ 2        ┆                                                          │
│ dev_fees.implied_volume (inf)      ┆ —        ┆ no fee inverted; 2 fees not invertible (no fee in force) │
│ dev_fees.with_fee_volume           ┆ 0 sats   ┆                                                          │
│ dev_fees.implied_vs_observed (inf) ┆ —        ┆ no inverted fee names a known order                      │
└────────────────────────────────────┴──────────┴──────────────────────────────────────────────────────────┘
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
unless overridden per instance). `with_fee_volume` is the observed sats of
the orders that are still known, and `implied_vs_observed` is
`implied ÷ observed − 1` over those same orders: a positive figure means
the instance forwards more than assumed. Here every fee names an order the
relays no longer have and the instance's fee at the time is unknown, so
nothing can be inverted and the rows say so.

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
rebuild: 22 events replayed (0 unreadable), 8 orders and 5 disputes projected
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
request. The aggregation layer is a separate crate, `crates/stats`, with no
I/O dependencies — a compile error, not a convention — so that the same
figures can be served over HTTP later without touching them.
