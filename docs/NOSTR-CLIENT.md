# bestiario — what a client can read

Status: draft (2026-08-28). Companion to `docs/NOSTR-PUBLICATION.md`, which
is the normative specification of the format. This document is the
**catalogue**: every document bestiario publishes, every figure inside it,
and what each one means — written for somebody building a Nostr client, a
dashboard or a site on top of it, who wants to know what is available
before reading a specification.

Section numbers written `PUB §N` refer to `docs/NOSTR-PUBLICATION.md`, and
`SPEC §N` to `docs/SPEC.md`. Where the two disagree with this document, they
are right and this one is stale — report it.

---

## 1. What this is

bestiario indexes the public Nostr events that Mostro instances publish —
orders, disputes, dev fees, instance self-descriptions — and computes
network statistics from them. It then publishes those statistics back to
Nostr as signed, addressable events.

So a client needs **no HTTP API and no backend**. It needs one relay
connection, one publisher pubkey, and the `d` values below.

**What you are trusting.** A signature, not a hostname. Every document is
signed by the publisher's key, so the same figures can be served from a
site, from IPFS, or bundled in an app, and a client that verifies the
signature is protected in all three. A client that renders unverified
content is not conformant (PUB §1).

**What the signature does not prove** is that the figures are correct — only
that bestiario published them. Every figure is labelled `observed` or
`inferred`, and that distinction must survive into what you render (§6).

---

## 2. The five-minute version

Everything is kind **30666**, addressable, from the publisher's pubkey.

```json
{ "kinds": [30666], "authors": ["<publisher pubkey>"], "#d": ["index"] }
```

1. **Fetch `index`.** Verify the signature. It lists every document of the
   current snapshot, each with the SHA-256 of its `payload`.
2. **Fetch what you render**, by `d`, several at a time — `#d` values are
   OR'd, so a panel of six documents is one `REQ`:

   ```json
   {
     "kinds": [30666],
     "authors": ["<publisher pubkey>"],
     "#d": ["summary:30d", "orders:24h", "market:30d", "instances:all"]
   }
   ```
3. **Check each one** against the hash the index named for it, then read
   `content` as JSON and render `payload`.
4. **Subscribe** to the same `d` values for live replacement. Relays keep
   only the newest event per `(pubkey, kind, d)`, so an update simply
   replaces what you have.

That is the whole protocol. The rest of this document is what is inside.

---

## 3. The address catalogue

A document's `d` tag is its name, and you construct it. The grammar is
normative (PUB §3): lowercase, exact match, a typo is a miss rather than a
fuzzy match.

### 3.1 The index

| `d` | What it is |
| --- | --- |
| `index` | The list of every published document, with hashes, revisions and the archive's coverage. The only address you have to know a priori. |

### 3.2 Window documents — `<report>:<window>`

Eight reports × five windows = forty documents, all of which exist.

| Report | What it answers |
| --- | --- |
| `summary` | The headline: created, completed, volume, active instances, open disputes. |
| `orders` | Order activity: created, completed, cancelled, completion and abandonment rates, the live book. |
| `volume` | Sats traded, ticket sizes, size distribution, buy/sell split, per-currency fiat totals, and a converted total in the reference currency. |
| `market` | Market structure: buy/sell pressure, premiums, market-price and range shares, currency and payment-method concentration. |
| `disputes` | Disputes opened and resolved, by status, initiator and outcome, plus the disputes still waiting for a solver right now. |
| `dev-fees` | Dev fees observed, their coverage and latency, and the volume they imply. |
| `instances` | One block per instance: its self-published profile, when it was first and last seen, how long it has been silent, and what it created in the window. **The only place a client learns that an instance exists.** |
| `compare` | One row per instance, side by side: completed, volume, completion rate, fee, dev fees, dispute rate, version. |

| Window | Span |
| --- | --- |
| `24h` | The last 24 hours |
| `7d` | The last 7 days |
| `30d` | The last 30 days |
| `90d` | The last 90 days |
| `all` | The whole archive |

Examples: `summary:30d`, `orders:24h`, `instances:all`, `compare:7d`.

A window does *not* end at the current clock. It ends at the archive's
ceiling — the timestamp of the latest event bestiario has stored — and the
payload's `range` says exactly where. This is deliberate: a window running
to the wall clock would count the minutes since the last indexed event as
minutes the network was idle (PUB §6.1).

### 3.3 Per-instance documents — `orders:<window>:i:<pubkey>`

For **every** instance the archive knows, at all five windows:

```
orders:24h:i:6320ee5edbaeb9a00d7c4768e472e277539aa993007c43d75ce00a38dff4d425
orders:30d:i:6320ee5edbaeb9a00d7c4768e472e277539aa993007c43d75ce00a38dff4d425
orders:all:i:6320ee5edbaeb9a00d7c4768e472e277539aa993007c43d75ce00a38dff4d425
```

The scope is the **full 64-character lowercase-hex pubkey**, never a prefix.
Get the pubkeys from the `instances` document (§9).

These carry that instance's order figures *and the same figures per
currency* — which instance trades which currencies, and how much — and that
cross exists in no other document. §7 is the worked example.

Only `orders` is published per instance. `volume:30d:i:…` and the rest are
valid addresses under the grammar but are **not published**; asking for one
gets you nothing.

### 3.4 Series partitions — `series:<report>:<resolution>:<bucket>`

Time series, in a compact columnar form, for the four reports that have a
shape over time: `orders`, `volume`, `dev-fees`, `disputes`.

| Resolution | Bucket | Example |
| --- | --- | --- |
| `daily` | A month, `YYYY-MM` | `series:orders:daily:2026-08` |
| `weekly` | A month, `YYYY-MM` | `series:volume:weekly:2026-08` |
| `monthly` | A year, `YYYY` | `series:disputes:monthly:2026` |

There is no range query in Nostr. To chart a span, **enumerate the
partitions that cover it** and ask for them in one `REQ`. Pick the
resolution from the span (PUB §9.2):

| Span requested | Resolution | Partitions |
| --- | --- | --- |
| < 90 days | `daily` | ≤ 4 |
| < 2 years | `weekly` | ≤ 24 |
| ≥ 2 years | `monthly` | ≤ 10 per decade |

The index's `resolutions` block tells you which buckets actually exist, so
you never ask for a month nobody published:

```json
"resolutions": {
  "daily":   { "from": "2026-02", "until": "2026-08" },
  "weekly":  { "from": "2026-02", "until": "2026-08" },
  "monthly": { "from": "2026",    "until": "2026" }
}
```

`summary`, `market`, `instances` and `compare` have **no** series. Their
window documents exist at every window regardless.

---

## 4. What a document looks like

### 4.1 The envelope

Every document except the index has the same outer shape (PUB §6):

```json
{
  "schema_version": 1,
  "snapshot_id": "20260827T030640Z",
  "generated_at": "2026-08-27T03:06:40+00:00",
  "revision": 2,
  "restated_at": "2026-08-27T03:06:40+00:00",
  "restated_because": "backfill",
  "payload": { "…": "the figures, and only the figures" }
}
```

- Everything outside `payload` describes the **run**; `payload` is the
  answer. Only `payload` is hashed, so "did this change" is a question
  about figures and not about clocks.
- `revision` starts at 1 and increases every time the figures move.
  `restated_at` / `restated_because` are present only above revision 1, and
  `restated_because` is one of `backfill`, `rebuild`, `schema`,
  `correction`. A client that has cached a document and sees a higher
  revision should surface that the figure changed, not silently swap it.

### 4.2 A window payload

```json
{
  "range": { "from": "2026-07-28T10:39:33+00:00", "until": "2026-08-26T10:39:33+00:00" },
  "metrics": [
    { "name": "orders.created",         "kind": "observed", "unit": "count", "value": 137 },
    { "name": "orders.completion_rate", "kind": "observed", "unit": "ratio", "value": 0.72 },
    { "name": "volume.in.USD.total",    "kind": "inferred", "unit": "fiat",
      "value": { "amount": 4180.5, "code": "USD" },
      "error": "rate snapshot at or before success_at; see SPEC §5" }
  ]
}
```

A flat list of records. Look figures up **by `name`**; do not rely on
position or on the list's length, both of which change as reports gain
figures.

### 4.3 A series payload

```json
{
  "period": { "from": "2026-08-01T00:00:00+00:00", "until": "2026-09-01T00:00:00+00:00" },
  "resolution": "daily",
  "columns": [
    { "name": "date",      "unit": "date" },
    { "name": "created",   "kind": "observed", "unit": "count" },
    { "name": "completed", "kind": "observed", "unit": "count" }
  ],
  "rows": [
    ["2026-08-01", 12, 7],
    ["2026-08-02", 0, 0],
    ["2026-08-03", null, null]
  ]
}
```

`kind` and `unit` are declared **once per column**, not per cell. The first
cell of every row is the bucket key. Rows are ascending, one per bucket,
none skipped. Column names are the metric names without the family prefix:
the `created` column of `series:orders:*` is `orders.created`.

The third row above is the important one — see §6.

### 4.4 The index

The one document with no envelope: nothing hashes the index, so its fields
sit at the top level.

```json
{
  "schema_version": 1,
  "snapshot_id": "20260827T030640Z",
  "generated_at": "2026-08-27T03:06:40+00:00",
  "publisher": { "name": "bestiario", "version": "0.2.0" },
  "coverage": {
    "first_event_at": "2026-02-14T11:20:03+00:00",
    "last_event_at":  "2026-08-26T10:39:33+00:00"
  },
  "resolutions": { "daily": { "from": "2026-02", "until": "2026-08" } },
  "documents": [
    { "d": "summary:30d", "hash": "3f9a…", "revision": 1,
      "updated_at": "2026-08-27T03:06:40+00:00" },
    { "d": "series:orders:daily:2026-01", "hash": "b710…", "revision": 2,
      "updated_at": "2026-08-27T03:06:40+00:00",
      "restated_at": "2026-08-27T03:06:40+00:00",
      "restated_because": "backfill" }
  ]
}
```

`coverage` is the archive's real extent, and it is what makes zeros
readable. `updated_at` is when the figures last changed, not when the
document was last sent.

### 4.5 Tags

| Tag | Indexed by relays | Value |
| --- | --- | --- |
| `d` | yes | The document address |
| `s` | yes | `snapshot_id` of the run that last computed the payload |
| `t` | yes | `bestiario` |
| `alt` | no | A human sentence, for clients that do not know the kind (NIP-31) |
| `resolution` | no | `daily` / `weekly` / `monthly`, series only |
| `period` | no | `<from>` `<until>`, RFC 3339, series only |
| `revision` | no | Integer |
| `schema_version` | no | Integer |

Use `created_at` — which is signed — to show the age of the data. Do not
trust `generated_at` for that; a publisher reporting on its own freshness
proves nothing.

---

## 5. Reading a metric

Four fields, and one trap.

| Field | Meaning |
| --- | --- |
| `name` | Dotted path. The same name means the same thing in every document. |
| `kind` | `observed` — counted straight from what instances published. `inferred` — derived from an assumption, and always accompanied by `error`. |
| `unit` | `count`, `sats`, `ratio`, `seconds`, `fiat`, `text`, `date`, or `missing`. |
| `value` | The figure. `null` when absent. |
| `error` | Present only on `inferred` figures: what makes the number uncertain. Make this reachable in the UI. |

**The trap:** when a figure is absent, `unit` is `"missing"` and `value` is
`null` — the natural unit is *not* reported. So `unit` tells you how to
format a value you have; it does not tell you a metric's type in general.
Key your formatting off the metric name, and treat `missing` as "no figure
this time".

Units in detail:

- `count` — an integer.
- `sats` — an integer number of satoshis.
- `ratio` — a fraction, `0.72` meaning 72%. Premiums are also ratios:
  `market.premium_p50` of `0.015` is a premium of 1.5%.
- `seconds` — an integer duration.
- `fiat` — an object, `{ "amount": 4180.5, "code": "USD" }`.
- `text` — a string. Some text figures are pre-formatted lists, noted below.
- `date` — a bucket key in a series, `2026-08-01` or `2026-W31` or
  `2026-08`.

---

## 6. Four rules a client must follow

**1. `null` is absence, never zero.** A bucket outside the archive's
coverage is `null` in *every* column, counts included. Rendering those as
zero draws a flat line at zero across a period the network was busy and
nobody had indexed yet — the single most misleading thing this data can be
made to say. Render a gap.

A bucket that *is* covered and saw nothing has a real `0` for its counts and
`null` for its rates. The difference is the whole point, and `coverage` in
the index is what lets you tell them apart.

**2. `inferred` must look different from `observed`.** An inferred figure
rests on an assumption — a dev fee share, a rate snapshot — and its `error`
string says which. Mark it visually and make the error reachable.

**3. Check the hash, not the snapshot id.** A snapshot is many events and
cannot be published atomically, so a client can catch one document from each
of two runs. The index is the authority: render a document only when the
SHA-256 of its `payload` matches the `hash` the index names for that `d`;
on a mismatch, mark the panel as updating and wait.

Do **not** additionally require the event's `s` tag to equal the index's
`snapshot_id`. A document whose figures have not changed is not
republished, so it legitimately carries an older `snapshot_id` — and those
are precisely the documents most certainly current.

**4. Show the age of the data.** From `created_at`, and warn past a
threshold you choose.

---

## 7. Orders by currency, per instance

This is the reason §3.3 exists, so here it is end to end.

**Goal:** a panel reading "Mostro — ARS 13, USD 2, BRL 1" beside
"MostroColombia — COP 23, ARS 2, VES 11".

**Step 1** — fetch `instances:30d` and read every instance's pubkey. The
metric names you want are the ones ending in `.pubkey`:

```
instances.Mostro (6320ee5e).pubkey           → "6320ee5edbae…d425"
instances.MostroColomBia🇨🇴 (2be7ca27).pubkey → "2be7ca274d2d…89a4"
```

**Step 2** — build one address per instance and ask for them together:

```json
{
  "kinds": [30666],
  "authors": ["<publisher pubkey>"],
  "#d": [
    "orders:30d:i:6320ee5edbaeb9a00d7c4768e472e277539aa993007c43d75ce00a38dff4d425",
    "orders:30d:i:2be7ca274d2df0abc141bc3e8472b953bb54090aae5c24a74de69b45f57e89a4"
  ]
}
```

**Step 3** — each payload holds the instance's totals and one block per
currency it traded:

```json
{
  "range": { "from": "…", "until": "…" },
  "metrics": [
    { "name": "orders.created",       "kind": "observed", "unit": "count", "value": 16 },
    { "name": "orders.open_now",      "kind": "observed", "unit": "count", "value": 4 },

    { "name": "orders.ARS.created",   "kind": "observed", "unit": "count", "value": 13 },
    { "name": "orders.ARS.completed", "kind": "observed", "unit": "count", "value": 9 },
    { "name": "orders.ARS.open_now",  "kind": "observed", "unit": "count", "value": 3 },

    { "name": "orders.USD.created",   "kind": "observed", "unit": "count", "value": 2 },
    { "name": "orders.BRL.created",   "kind": "observed", "unit": "count", "value": 1 }
  ]
}
```

Every metric matching `orders.<CODE>.created` is one bar of the chart. The
currency codes are the three-letter codes instances publish.

A currency has **no block at all** — rather than a block of zeros — when
every figure the block would carry is zero. So the currencies you see are
the ones that did something in this window, or have an order live right
now; enumerate them from the payload rather than assuming a fixed set, and
treat a missing code as zero on every count.

Each currency block carries the same nine figures as the instance total:

| Suffix | Unit | Meaning |
| --- | --- | --- |
| `created` | count | Orders created in the window |
| `completed` | count | Orders that reached `success` in the window |
| `canceled` | count | Orders that reached `canceled`, expiry included |
| `completion_rate` | ratio | `completed / (completed + canceled)` |
| `abandonment_rate` | ratio | Of those created, the share cancelled with no taker |
| `created_delta` | ratio | Growth against the previous window of the same length |
| `completed_delta` | ratio | The same for `completed` |
| `open_now` | count | `pending` orders still live — about *now*, not the window |
| `in_progress_now` | count | Orders with a taker — also about *now* |

**These sum.** An order names exactly one currency and belongs to exactly
one instance, so the currency blocks partition the instance's orders and the
instances partition the network's. Add currencies for an instance total; add
instances for the network's mix by currency. Nothing is double-counted.

That is *not* true of payment methods, which an order may name several of at
once and which are attributed to each rather than divided between them —
which is why `market.method_top3_by_volume` adds up to more than the volume
traded, and why there are no per-method blocks.

**For the network's mix by currency you do not need these at all.**
`orders:<window>` carries `orders.<CODE>.created`, `.completed`, `.canceled`
and `.open_now` for every currency the network traded — one document, one
`REQ`. The per-instance documents are for the *cross*: which instance is
behind which currency. The two agree by construction; §7's summing rule is
what makes that true, and the publisher asserts it in its own tests.

**Volume by currency, network-wide,** is a different document again:
`volume:<window>` carries `volume.fiat.<CODE>.total` and friends (§8.3).
Careful — that block counts only completed orders with a **fixed** fiat
amount, so range orders are missing from it, and its totals are in fiat, not
sats. For "how many successful orders in ARS", use
`orders.ARS.completed`, which counts them all. Volume in **sats** per
currency is not published at any scope.

---

## 8. The complete metric catalogue

Every figure, by document. `<CODE>` is a three-letter currency code,
`<instance>` an instance key (§9), `<n>` a rank starting at 1.

### 8.1 `summary:<window>`

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `summary.created` | observed | count | Orders created in the window |
| `summary.completed` | observed | count | Orders completed in the window |
| `summary.completion_rate` | observed | ratio | Completed over completed + cancelled |
| `summary.volume_sats` | observed | sats | Sats traded |
| `summary.active_instances` | observed | count | Instances that created an order |
| `summary.top_fiat` | observed | text | Leading currencies, pre-formatted |
| `summary.top_methods` | observed | text | Leading payment methods, pre-formatted |
| `summary.open_disputes` | observed | count | Disputes open right now |

### 8.2 `orders:<window>` and `orders:<window>:i:<pubkey>`

The nine figures of §7's table, under `orders.*`, in both scopes.

Both scopes also break those orders down by currency, to different depths:

| | Per currency |
| --- | --- |
| `orders:<window>` (network) | **Four counts**: `orders.<CODE>.created`, `.completed`, `.canceled`, `.open_now` |
| `orders:<window>:i:<pubkey>` | **All nine figures**: the whole block again as `orders.<CODE>.*` |

The network document is the shallower one because the network's currency
list has no ceiling while the document size does (PUB §6.1.1). The four it
carries are the ones that *sum*, which is what a client actually does with
them; `completion_rate` per currency is `completed / (completed + canceled)`
and derives from two of them, and the rest is a per-instance question.

Series columns (`series:orders:*`): `created`, `completed`, `canceled`,
`completion_rate`, `abandonment_rate` — and **no column per currency**, at
any resolution. The two `_now` figures and the two `_delta` ones are absent
by design too: one kind is about the clock rather than the window, and the
other is already a change.

### 8.3 `volume:<window>`

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `volume.sats` | observed | sats | Sats traded in the window |
| `volume.completed` | observed | count | Orders behind that figure |
| `volume.ticket_avg` | observed | sats | Mean trade size |
| `volume.ticket_p50` | observed | sats | Median trade size |
| `volume.ticket_p90` | observed | sats | 90th percentile trade size |
| `volume.largest` | observed | sats | Largest single trade |
| `volume.size.lt_10k` | observed | count | Trades under 10 000 sats |
| `volume.size.10k_50k` | observed | count | 10 000 – 50 000 sats |
| `volume.size.50k_200k` | observed | count | 50 000 – 200 000 sats |
| `volume.size.200k_1m` | observed | count | 200 000 – 1 000 000 sats |
| `volume.size.gt_1m` | observed | count | Over 1 000 000 sats |
| `volume.buy_sats` | observed | sats | Volume of completed buy orders |
| `volume.sell_sats` | observed | sats | Volume of completed sell orders |
| `volume.fiat.<CODE>.total` | observed | fiat | Fiat traded in that currency |
| `volume.fiat.<CODE>.orders` | observed | count | Fixed-amount orders behind it, and the denominator of the tickets |
| `volume.fiat.<CODE>.sats` | observed | sats | Sats that currency moved. Over **every** completed order in it, range orders included, so these add up to `volume.sats` |
| `volume.fiat.<CODE>.completed` | observed | count | Completed orders behind the sats — never below `orders`, and above it when a range order completed |
| `volume.fiat.<CODE>.ticket_avg` | observed | fiat | Mean ticket in that currency |
| `volume.fiat.<CODE>.ticket_p50` | observed | fiat | Median ticket |
| `volume.fiat.<CODE>.ticket_p90` | observed | fiat | 90th percentile ticket |
| `volume.in.<CODE>.total` | **inferred** | fiat | Everything priced into one reference currency |
| `volume.in.<CODE>.orders` | **inferred** | count | Orders it could price |
| `volume.in.<CODE>.unpriced_sats` | **inferred** | sats | Sats it could not price, for want of a rate |
| `volume.in.<CODE>.rate_age_max` | **inferred** | seconds | Age of the oldest rate used |

`volume.fiat.*` is observed — instances published those amounts. Two
populations live in the block and each carries its own count: `total` and the
tickets are the fixed-amount orders, since a range order names no single fiat
amount; `sats` and `completed` are every completed order in the currency.
`volume.in.*` is inferred: it converts sats at a rate snapshot, and
`unpriced_sats` is how much of the window it could not speak for. Render the
two differently.

### 8.4 `market:<window>`

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `market.orders` | observed | count | Orders created in the window |
| `market.buy_orders_share` | observed | ratio | Buys over orders created |
| `market.buy_volume_share` | observed | ratio | Buy sats over sats completed |
| `market.premium_avg` | observed | ratio | Mean premium over completed orders |
| `market.premium_p50` | observed | ratio | Median premium |
| `market.premium_p50_buy` | observed | ratio | Median premium, buys |
| `market.premium_p50_sell` | observed | ratio | Median premium, sells |
| `market.premium_spread` | observed | ratio | `p50(sell) − p50(buy)` |
| `market.market_price_share` | observed | ratio | Orders priced at market |
| `market.range_share` | observed | ratio | Orders published as a range |
| `market.range_width_avg` | observed | ratio | Mean `(max − min) / max` of ranges |
| `market.fiat_top3_by_orders` | observed | text | `ARS 13, USD 2, BRL 1` — the top three only |
| `market.fiat_top3_orders_share` | observed | ratio | Share of orders held by the top three |
| `market.fiat_hhi_orders` | observed | ratio | Herfindahl index, 1 for a monopoly |
| `market.fiat_top3_by_volume` | observed | text | The same by volume |
| `market.fiat_top3_volume_share` | observed | ratio | |
| `market.fiat_hhi_volume` | observed | ratio | |
| `market.method_top3_by_orders` | observed | text | Leading payment methods |
| `market.method_top3_by_volume` | observed | text | The same by volume |
| `market.new_fiats` | observed | text | Currencies whose first order ever fell in the window |
| `market.new_methods` | observed | text | Payment methods likewise |

The `top3` figures are **pre-formatted strings of the top three**, for a
text cell. For a machine-readable currency breakdown, use the per-instance
`orders` documents (§7) and sum them.

### 8.5 `disputes:<window>`

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `disputes.opened` | observed | count | Disputes opened in the window |
| `disputes.status.initiated` | observed | count | By latest status |
| `disputes.status.in_progress` | observed | count | |
| `disputes.status.seller_refunded` | observed | count | |
| `disputes.status.settled` | observed | count | |
| `disputes.status.released` | observed | count | |
| `disputes.initiator.buyer` | observed | ratio | Share opened by the buyer |
| `disputes.initiator.seller` | observed | ratio | Share opened by the seller |
| `disputes.rate` | observed | ratio | Disputes per order |
| `disputes.resolved` | observed | count | Disputes resolved in the window |
| `disputes.outcome.seller_refunded` | observed | ratio | Share of resolutions |
| `disputes.outcome.settled` | observed | ratio | |
| `disputes.outcome.released` | observed | ratio | |
| `disputes.resolution_p50` | observed | seconds | Median time to resolution |
| `disputes.resolution_p90` | observed | seconds | 90th percentile |
| `disputes.open_now` | observed | count | Still `initiated` right now — waiting for a solver |
| `disputes.open.<n>.id` | observed | text | The waiting book, listed |
| `disputes.open.<n>.age` | observed | seconds | How long it has been waiting |

`disputes.open.<n>.age` is `now − opened_at`, so it moves between two runs
over an unchanged archive — which is why the five `disputes` documents are
republished every run.

### 8.6 `dev-fees:<window>`

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `dev_fees.total_sats` | observed | sats | Dev fees seen |
| `dev_fees.paid` | observed | count | Fee events counted |
| `dev_fees.coverage` | observed | ratio | Completed orders with a fee attached |
| `dev_fees.latency_p50` | observed | seconds | Median delay between order and fee |
| `dev_fees.latency_p90` | observed | seconds | 90th percentile |
| `dev_fees.duplicates` | observed | count | Orders with more than one fee |
| `dev_fees.orphans` | observed | count | Fees naming an order not indexed |
| `dev_fees.implied_volume` | **inferred** | sats | Volume the fees imply, at an assumed fee share |
| `dev_fees.with_fee_volume` | observed | sats | Observed volume of the orders that paid one |
| `dev_fees.implied_vs_observed` | **inferred** | ratio | The two against each other |

The inferred pair rests on a configured per-instance fee percentage. It is
an estimate of trade the indexer cannot see directly, and should be labelled
as one.

### 8.7 `instances:<window>`

One block per instance, keyed as §9 describes.

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `instances.<instance>.pubkey` | observed | text | The 64-hex pubkey — **use this to build addresses** |
| `instances.<instance>.name` | observed | text | The name it publishes, if any |
| `instances.<instance>.mostro_version` | observed | text | |
| `instances.<instance>.protocol_version` | observed | text | |
| `instances.<instance>.fee` | observed | ratio | Per-side fee, `0.006` being 0.6% |
| `instances.<instance>.min_order` | observed | sats | |
| `instances.<instance>.max_order` | observed | sats | |
| `instances.<instance>.fiat` | observed | text | Currencies it *lists*, comma-separated |
| `instances.<instance>.ln_networks` | observed | text | Comma-separated |
| `instances.<instance>.bond` | observed | text | `enabled` / `disabled` |
| `instances.<instance>.first_seen` | observed | text | RFC 3339 |
| `instances.<instance>.last_seen` | observed | text | RFC 3339 |
| `instances.<instance>.silent_for` | observed | seconds | Since its last event of any kind |
| `instances.<instance>.silent` | observed | text | `yes` past a week of silence, `no` otherwise |
| `instances.<instance>.created` | observed | count | Orders it created in the window |

`fiat` is what the instance *advertises* in its self-description. What it
actually traded is the per-currency blocks of §7, and the two routinely
disagree — an instance may list ten currencies and trade two.

`silent_for` is clock-relative, which is why these five documents are
republished every run.

### 8.8 `compare:<window>`

One row per instance, for a league table.

| Metric | Kind | Unit | Meaning |
| --- | --- | --- | --- |
| `compare.<instance>.completed` | observed | count | Orders completed |
| `compare.<instance>.volume_sats` | observed | sats | Volume |
| `compare.<instance>.completion_rate` | observed | ratio | |
| `compare.<instance>.fee` | observed | ratio | Published fee |
| `compare.<instance>.dev_fees_sats` | observed | sats | Dev fees observed from it |
| `compare.<instance>.dispute_rate` | observed | ratio | |
| `compare.<instance>.version` | observed | text | Mostro version |

### 8.9 Series columns

Column names are the metric names above with the family prefix stripped.

- `series:orders:*` — `created`, `completed`, `canceled`, `completion_rate`,
  `abandonment_rate`
- `series:volume:*` — `sats`, `completed`, `ticket_avg`, `ticket_p50`,
  `ticket_p90`, `largest`, the five `size.*`, `buy_sats`, `sell_sats`, the
  `fiat.<CODE>.*` group and the `in.<CODE>.*` group
- `series:dev-fees:*` — `total_sats`, `paid`, `coverage`, `latency_p50`,
  `latency_p90`, `duplicates`, `orphans`, `implied_volume`,
  `with_fee_volume`, `implied_vs_observed`
- `series:disputes:*` — `opened`, the five `status.*`, the two
  `initiator.*`, `rate`, `resolved`, the three `outcome.*`,
  `resolution_p50`, `resolution_p90`

Which columns a partition actually declares depends on what the archive held
over that period — a currency traded in no month of 2026 has no
`fiat.<CODE>.total` column in 2026's partitions. Read `columns` rather than
assuming.

---

## 9. Joining documents: labels and pubkeys

A wart worth knowing before you write a parser.

In `instances.*` and `compare.*`, the key inside the metric name is the
instance's **display label**, not its pubkey:

```
instances.Mostro (6320ee5e).created
compare.MostroColomBia🇨🇴 (2be7ca27).volume_sats
instances.807fdded0e6c21acbd19103e6ddc503bbef45078feaaa6c749fae7e7c00ef88f.created
```

The label is `name (first 8 hex of the pubkey)` when the instance publishes a
name, and the bare 64-hex pubkey when it does not. It is unique either way.

Two consequences:

1. **Do not split metric names on `.` to recover the instance.** Labels
   contain spaces and emoji, and can contain dots — an instance named
   `mostro.example.com` yields
   `instances.mostro.example.com (abc12345).created`. Take the label as
   everything between the known prefix and the known suffix instead: for a
   name of the form `instances.<label>.<field>`, strip `instances.` from the
   front and `.<field>` from the end.

2. **To go from a block to a document address**, read the `.pubkey` row of
   that block. That is the only reliable bridge between the `instances` /
   `compare` blocks and the `orders:<window>:i:<pubkey>` addresses. The
   robust enumeration is: take every metric whose name starts with
   `instances.` and ends with `.pubkey`; its `value` is the pubkey and the
   middle is the label key for that instance's other rows.

`compare` gives you labels but no pubkeys, so join it to `instances` of the
same window by label.

---

## 10. Recipes

| Panel | Documents |
| --- | --- |
| Network headline | `summary:24h`, `summary:30d` |
| Orders over the last month, daily | `series:orders:daily:<this month>` and the one before |
| Volume over a year | `series:volume:monthly:<year>` |
| Instance directory or map | `instances:all` |
| League table | `compare:30d`, joined to `instances:30d` by label for pubkeys |
| **Currencies per instance** | `instances:30d` for the pubkeys, then one `orders:30d:i:<pubkey>` per instance |
| Network currency mix by orders | `orders:30d`, the `orders.<CODE>.*` rows — one document |
| Successful orders by currency | `orders.<CODE>.completed`, network-wide or per instance |
| Network currency mix by fiat volume | `volume:30d`, the `volume.fiat.<CODE>.*` rows |
| Market structure | `market:30d` |
| Live book | `orders:24h`, rows `orders.open_now` and `orders.in_progress_now` |
| Dispute health | `disputes:30d`, plus `disputes.open.<n>.*` for what is open now |

---

## 11. What is not published

So you do not go looking:

- **Per-instance series.** `series:orders:daily:2026-08:i:<pubkey>` parses
  but is not published (PUB §13.4).
- **Per-instance documents other than `orders`.** No `volume:…:i:…`, no
  `market:…:i:…`.
- **Network-scoped documents.** `orders:30d:n:mainnet` parses; nothing
  publishes one today.
- **Individual orders, disputes or events.** bestiario publishes statistics,
  not a mirror of the Mostro events — those are on the relays already, under
  kinds 38383, 38386, 8383 and 38385.
- **Anything about counterparties.** No pubkeys of takers or makers, no
  identities. Only instances are named.

---

## 12. Where to look next

- `docs/NOSTR-PUBLICATION.md` — the normative format: addressing, hashing,
  atomicity, restatement, size limits, and the conformant client algorithm.
- `docs/SPEC.md` — what every figure means and how it is computed, including
  the `observed` / `inferred` rule (SPEC §5) and the report definitions
  (SPEC §6).
- `README.md` — running the indexer, and the same figures on the command
  line, which is the fastest way to see what a document will contain:
  `bestiario --instance <name> stats orders --by fiat --json` prints exactly
  what `orders:<window>:i:<pubkey>` carries.
