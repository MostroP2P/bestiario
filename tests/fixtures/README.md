# Event fixtures

Real, signed events captured from `wss://relay.mostro.network` on
**2026-08-26** with [`nak`](https://github.com/fiatjaf/nak). Nothing here is
synthetic: every file is an event some instance actually published, with its
original signature intact, and `tests/fixtures.rs` verifies that on every run.

They exist so that parser tests in phase 1 are written against what the network
publishes rather than against what `docs/SPEC.md` says it publishes. Those two
turned out to differ, in ways described below.

## How they were captured

```sh
nak req -k 38383 -l 200 wss://relay.mostro.network > orders.jsonl
nak req -k 8383  -l 50  wss://relay.mostro.network > dev-fees.jsonl
nak req -k 38386 -l 30  wss://relay.mostro.network > disputes.jsonl
nak req -k 38385 -l 20  wss://relay.mostro.network > info.jsonl
nak req -k 30078 -d mostro-rates -l 10 wss://relay.mostro.network > rates.jsonl
nak req -k 10002 -a <pubkey> -l 10 wss://relay.mostro.network > relay-lists.jsonl
```

The sample was 200 orders across 16 instances, plus 20 instance-info events
across 20 instances. Files were then selected to cover the shapes a parser has
to survive, not to be representative of volume.

## What the sample showed that the spec does not say

Recorded here because each one changes how a parser has to be written. The
spec has been amended; this is the evidence.

### `y` is a platform tag, and not every platform is Mostro

`docs/SPEC.md` §2.1 described `y` as `["mostro", instance_name?]`. In practice
its first value identifies the **platform**, and the Mostro relays carry
NIP-69 orders from several:

| `y[0]` | Instances in the sample | Orders |
|---|---|---|
| `mostro` | 11 | 172 |
| `telegram` | 1 | 16 |
| `hodlhodl` | 1 | 7 |
| `Bitblik` | 2 | 3 |
| `Bitway` | 1 | 2 |

bestiario measures the Mostro network, so it has to filter on this. Without
it, `accept_unknown_instances = true` would silently fold four other
platforms' activity into the Mostro figures. The
`other_platform_*.json` fixtures exist to test that they are rejected.

### Telling platforms apart changes what looks optional

Across the sample, `expires_at` appeared on 172 of 200 orders — which reads as
"optional, tolerate its absence". Split by platform, the picture is different:
**every one of the 172 Mostro orders published it**, and all 28 orders that
omitted it came from another platform.

A corpus that mixed platforms would have led to a parser written to tolerate a
missing field that Mostro always sends. `every_mostro_order_publishes_expires_at`
pins this down.

### A pending range order does not always publish `amt = 0`

`pending_range_with_fixed_sats.json` is `pending` with `fa = [5, 350]` **and**
`amt = 6135`: the maker fixed the sats and left the fiat to the taker, which is
the mirror image of the market-price order that fixes the fiat and leaves `amt`
at 0. So `fa` having two values says nothing about `amt`, and neither field can
be used to infer the other. The fixture was captured as `pending_fixed_amount`
and renamed once the parser was written against it.

### Eight of the twenty-two Mostro instances publish no name

Counting across all six captures, 22 distinct pubkeys publish with
`y[0] = "mostro"`. Eight of them never send a second value — `y = ["mostro"]`
with nothing after it — and a ninth sends one on some kinds and not on others.
This settles the open question in `docs/SPEC.md` §14: it is not a rare edge
case, it is a third of the network.
Reports have to render an instance with no name, and `--instance` has to
resolve one by pubkey alone.

The name is also not published on every kind by the same instance: `b3626fe9…`
sends `["mostro", "MostrAR 🇦🇷"]` on its orders and `["mostro"]` on its
disputes. So the name has to be collected from whichever event carries it,
which is what `instances.name_seen_at` and the `instance_names` history are
for.

### Tags the spec does not list

Present on real events, currently unparsed. Kept in the fixtures so that
`events.raw_json` retains them and a later phase can use them without
re-capturing:

- **38383**: `layer` (always `lightning` in the sample), `expiration`
  (NIP-40), `source` (a `mostro:` URI with relay hints), `name`, `bond`,
  `reserved_at`, `created_at`, `paid_at`, `category`, `taker_fees`
- **38385**: a much larger set than §2.4 lists — `expiration_hours`,
  `expiration_seconds`, `max_orders_per_response`, `pow`, `pow_first_contact`,
  `hold_invoice_expiration_window`, `hold_invoice_cltv_delta`,
  `invoice_expiration_window`, `lnd_version`, `lnd_node_pubkey`,
  `lnd_commit_hash`, `lnd_node_alias`, `lnd_chains`, `lnd_uris`
- **38386**, **8383**, **30078**: match the spec, plus NIP-40 `expiration`

`protocol_version` is absent from 2 of the 20 instance-info events, so it is
optional in a way §2.4 does not say.

## The corpus

### Kind 38383

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `canceled.json` | `00037abd…` | Mostro Brasil | 1787740613 |
| `in_progress.json` | `82fa8cb9…` | Mostro | 1787740745 |
| `other_platform_bitway.json` | `1d6b0525…` | Bitway | 1787734938 |
| `other_platform_hodlhodl.json` | `273e7880…` | hodlhodl | 1787738113 |
| `other_platform_telegram.json` | `a852e1c6…` | NOVARUZBOT🦫 | 1787737406 |
| `pending_market_price.json` | `82fa8cb9…` | Mostro | 1787738870 |
| `pending_multiple_payment_methods.json` | `0000cc02…` | NostroMostro 🇪🇸 | 1787737743 |
| `pending_range.json` | `82fa8cb9…` | Mostro | 1787740678 |
| `pending_range_with_fixed_sats.json` | `82fa8cb9…` | Mostro | 1787723816 |
| `success.json` | `17b520bd…` | Fostro testing | 1787725716 |
| `with_maker_rating.json` | `00037abd…` | Mostro Brasil | 1787738626 |

### Kind 8383

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `another_instance.json` | `00007cb3…` | Mostro ₿oliviano🇧🇴 | 1787720776 |
| `typical.json` | `82fa8cb9…` | Mostro | 1787721916 |

### Kind 38386

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `status_in_progress.json` | `82fa8cb9…` | Mostro | 1787621078 |
| `status_initiated.json` | `00000235…` | Kmbalache 🇨🇺 | 1787533512 |
| `status_seller_refunded.json` | `00000978…` | MostroColomBia🇨🇴 | 1787712948 |
| `status_settled.json` | `00000978…` | MostroColomBia🇨🇴 | 1787698230 |
| `without_instance_name.json` | `b3626fe9…` | mostro | 1787517755 |

### Kind 38385

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `typical.json` | `0000cc02…` | NostroMostro 🇪🇸 | 1787740744 |
| `with_bond_policy.json` | `ef7d11a2…` | Sovereign Mostro VgWs | 1787740658 |
| `without_instance_name.json` | `f9436271…` | mostro | 1787740737 |
| `without_protocol_version.json` | `560795c6…` | mostro | 1787740684 |
| `zero_fee.json` | `c945e463…` | mostro | 1787740722 |

### Kind 30078

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `another_instance.json` | `000009ee…` | — | 1787740767 |
| `typical.json` | `82fa8cb9…` | — | 1787740773 |

### Kind 10002

| File | Instance | `y` | `created_at` |
|---|---|---|---|
| `another_instance.json` | `82fa8cb9…` | — | 1787740196 |
| `typical.json` | `00037abd…` | — | 1787740760 |
## Adding to it

Capture with `nak`, keep the signature intact, and file it under its kind.
`tests/fixtures.rs` will reject a file that is malformed, unsigned, in the
wrong directory, or a duplicate of one already present.
