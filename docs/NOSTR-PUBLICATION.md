# bestiario — Nostr publication

Status: draft v0.2 (2026-08-28). Companion to `docs/SPEC.md`, which stays the
source of truth for schema, metrics and report formats. This document
specifies one feature in full — publishing the computed figures as signed
Nostr events — and is referenced from SPEC §13 as a phase of its own.

This is the normative format. `docs/NOSTR-CLIENT.md` is the catalogue built
on top of it: every document that is actually published and every figure
inside it, written for somebody building a client rather than implementing
the format.

Section numbers of the form §N without a document name refer to this
document. References into the main specification are written `SPEC §N`.

## 1. Purpose and trust model

The daemon publishes the figures it has computed as signed Nostr events, so
that a client — the static site at `mostro.world`, a mobile app, a
third-party dashboard — can read them without an HTTP API and without
trusting whoever serves the page.

The consequence worth stating plainly: a reader trusts **one pubkey**, not a
host. A mirror of the site on another domain, on IPFS, or bundled in an app
shows the same figures or fails signature verification. The publisher's
pubkey is the root of trust and MUST be verified by every client; a client
that renders unverified content is not conformant.

What this does *not* establish is that the figures are correct. The signature
proves bestiario published them, nothing more. The `observed` / `inferred`
distinction of SPEC §5 travels with the data (§6) and MUST survive into any
rendering.

### 1.1 The relay is not the archive

Relays may prune, may enforce retention, and may refuse to store an
ever-growing set of documents. The SQLite archive remains the sole source of
truth, exactly as it is for the raw Mostro events bestiario indexes. Nostr is
a **distribution** channel here, not storage.

It follows that the daemon MUST be able to reconstruct and republish every
document it has ever published, from the archive alone, with no state kept
outside it (§9.3).

## 2. Event kind

All documents use a single addressable kind:

```
30666
```

Addressable (30000 ≤ kind < 40000): relays keep only the latest event per
`(pubkey, kind, d)`, so a republication supersedes its predecessor and the
archive on the relay does not grow with revisions.

One kind for every document type, with the `d` tag carrying all
discrimination. The alternative — a kind per report — buys a coarse filter
that `#d` already provides, and spends kind numbers that are a shared global
resource. The cost of the single-kind choice is that a client cannot
subscribe to "any bestiario document" and get only the index; it must name
the `d` values it wants, which it must do anyway (§4.1).

30666 is unassigned in the kind table of `nostr-protocol/nips` as of
2026-08-27; the neighbouring assignments are 30617/30618 (repository
announcements, NIP-34) and 30818/30819 (wiki, NIP-54). That table records
only what was registered, so an unregistered application kind may exist in
the wild — the risk is a client of *another* application mistaking bestiario
documents for its own, which the `d` grammar of §3 and the `alt` tag make
recoverable.

The number is `30000 + 666`. A bestiary catalogues monsters, *mostro* is
Italian for monster, and 666 is the number of the beast: the one candidate a
reader remembers without consulting this document.

MostroP2P SHOULD register 30666 upstream, as it did for 38383 and 38386, once
the format below stops moving.

Per NIP-31 every event carries an `alt` tag, so clients that do not know the
kind can say something truthful about it rather than rendering JSON.

## 3. Addressing: the `d` tag

The `d` value is the document's stable name. It is the only thing a client
needs to construct in order to fetch a document, so its grammar is normative
and MUST NOT be extended without a `schema_version` bump.

```
d          = index-doc / window-doc / series-doc

index-doc  = "index" [ ":" year ]

window-doc = report ":" window [ scope ]

series-doc = "series:" report ":" resolution ":" bucket [ scope ]

report     = "summary" / "orders" / "volume" / "market" / "disputes"
           / "dev-fees" / "instances" / "compare"

window     = "24h" / "7d" / "30d" / "90d" / "all"

resolution = "daily" / "weekly" / "monthly"

bucket     = YYYY "-" MM        ; for daily and weekly partitions
           / YYYY               ; for monthly partitions

scope      = ":i:" pubkey       ; one instance, 64 lowercase hex
           / ":n:" network      ; one network, e.g. "mainnet"

year       = YYYY
```

Examples:

```
index
summary:30d
orders:7d
orders:30d:i:6320ee5e…d425
volume:30d:n:mainnet
series:orders:daily:2026-01
series:volume:monthly:2026
series:orders:daily:2026-01:i:6320ee5e…d425
```

The instance scope uses the full pubkey, not a prefix. A prefix is a
collision waiting to be found by whoever wants to find it, and `d` values are
not length-constrained.

Rules:

- `d` values are lowercase and MUST match the grammar exactly. A client
  constructs them; a typo must be a miss, not a fuzzy match.
- Buckets are UTC. Weekly partitions are grouped into the month their **first
  day** falls in, so a week spanning a month boundary lives in one partition
  only.
- Windows are relative to the publishing moment, so window documents are
  perpetually restated by design. Series partitions are absolute and change
  only under restatement (§8).

## 4. Filters

### 4.1 Fetching a range

Nostr tag filters match on equality; there is no range predicate, and relays
index only single-letter tags. A time range is therefore fetched by
**enumerating the partitions that cover it**, which is why the index exists
(§5).

Fourteen months of daily series, in one subscription:

```json
{
  "kinds": [30666],
  "authors": ["<publisher pubkey>"],
  "#d": [
    "series:orders:daily:2026-01",
    "series:orders:daily:2026-02",
    "…",
    "series:orders:daily:2027-02"
  ]
}
```

Multiple values in a tag filter are OR'd, so the cost is one `REQ` and N
events, N being the number of partitions, not the number of data points.

### 4.2 Live updates

A client subscribes to the hot documents it renders and receives replacements
as they are published. There is no polling and no `generated_at` to trust:
`created_at` is signed, so staleness is computed by the client from the event
itself rather than reported by the publisher about itself.

## 5. The index

`d = index`. The first document a client fetches and the only `d` it needs to
know a priori, together with the publisher pubkey.

```json
{
  "schema_version": 1,
  "snapshot_id": "01J8Z…",
  "generated_at": "2026-08-27T03:05:00Z",
  "publisher": { "name": "bestiario", "version": "0.4.0" },
  "coverage": {
    "first_event_at": "2026-02-14T11:20:03Z",
    "last_event_at": "2026-08-27T03:04:51Z"
  },
  "resolutions": {
    "daily":   { "from": "2026-02", "until": "2026-08" },
    "monthly": { "from": "2026",    "until": "2026" }
  },
  "documents": [
    {
      "d": "summary:30d",
      "hash": "3f9a…",
      "revision": 1,
      "updated_at": "2026-08-27T03:05:00Z"
    },
    {
      "d": "series:orders:daily:2026-01",
      "hash": "b710…",
      "revision": 2,
      "updated_at": "2026-08-27T03:05:00Z",
      "restated_at": "2026-08-27T03:05:00Z",
      "restated_because": "backfill"
    }
  ]
}
```

The index answers three questions, each of which a client cannot answer
otherwise:

1. **What exists.** Which partitions were published, at which resolutions,
   from when. Without it a client guesses and requests months that were never
   published, and cannot distinguish "no data" from "not published".
2. **What changed.** `hash` is the SHA-256 of the document's `payload`
   (§6), hex, lowercase — the figures, not the envelope around them. A client
   compares it against what it has cached and fetches only the differences.
   Closed partitions are otherwise indistinguishable from restated ones.

   Deliberately not the whole `content`: `snapshot_id` and `generated_at`
   differ on every run, so a hash over them would make every closed
   partition a new hash, a new revision and a new signature every time the
   daemon runs — and the skip of §8, the cache of §10 and the whole point of
   `revision` would be dead letters. What a reader wants to know is whether
   the *figures* moved.

   `updated_at` is when the payload last changed, not when the document was
   last published, for the same reason.
3. **What is current.** `snapshot_id` is the atomicity token of §7.

`coverage` states the archive's real extent. A client MUST NOT render a
period outside it as zero; see §6.3.

### 5.1 Index growth

The index grows with the number of partitions: roughly one entry per report
per month. It also grows with the *network*, at five entries per instance for
the scoped documents of §6.1.1 — a little under a kilobyte each, so a dozen
instances is already the larger half of the index. It will eventually
approach the size limit of §9.1. When it does, it shards by year —
`index:2026`, `index:2027` — with the unqualified `index` listing the hot
documents, the resolutions available, and the year shards. The client
algorithm (§10) is written so that this change is additive.

## 6. Document formats

Two shapes, chosen by what the document holds. Both are JSON in `content`,
UTF-8, uncompressed and not base64-encoded: an event that is unreadable in a
generic Nostr client throws away most of the reason to publish on Nostr.

Both are also split the same way, and the split is load-bearing:

```json
{
  "schema_version": 1,
  "snapshot_id": "01J8Z…",
  "generated_at": "2026-08-27T03:05:00Z",
  "revision": 2,
  "payload": { "…": "the figures, and only the figures" }
}
```

Everything outside `payload` describes the *run*: which publication computed
this, when, and how many times the figures have moved. `payload` is the
answer itself. Only `payload` is hashed (§5), so "did this document change"
is a question about figures and not about clocks — which is what makes a
closed partition cacheable, and what keeps the daemon from re-signing a year
of history every night.

The index of §5 is the one document with no such split: nothing hashes it —
it is what the hashes are *in* — and it is republished on every run by
definition, since naming the current snapshot is its whole job.

`payload` is serialised deterministically: the same figures produce the same
bytes, so the hash is a property of the answer rather than of the serialiser
that happened to run. Field order is the order given below, and a
floating-point figure is rendered the way the JSON report of SPEC §10 renders
it. Two implementations that disagree here produce two hashes for one answer,
which is a bug in the one that deviates.

### 6.1 Window documents

Every report named in the grammar of §3 has window documents, at every
window, whether or not it has a series family. Having no shape over time is a
fact about a report's *series* and says nothing about its windows: `summary`,
`market`, `instances` and `compare` are views over the same archive and are
answers to a question a client asks by window like any other. A client that
constructs `instances:30d` from §3 and gets a miss cannot tell that from a
relay withholding the document, which is the confusion §5 exists to end.

`instances` in particular is the only place a client learns that an instance
exists at all — its pubkey, its name, the currencies it lists — and no other
document names one. A map of the network cannot be drawn without it.

`payload` is the report envelope already specified in SPEC §10 minus its
`generated_at`, which belongs to the run: `range`, and `metrics` as one flat
record per figure with `name`, `kind`, `unit`, `value`. Those records are
carried verbatim — no second format for the same thing.

`range` ends at the archive's ceiling — the `created_at` of its latest stored
event, clamped to the run's clock — and not at the clock itself. `all`
likewise begins at the archive's floor. A window running to the publishing
moment would count the stretch between the last stored event and `now` as a
period the network was idle, when what the archive knows about that stretch
is nothing: the flat line at zero §6.3 nulls a bucket to avoid, drawn inside
a window instead of across one. An ingest an hour behind would publish an
hour of quiet that did not happen.

It is also what lets §8 hold. `range` is inside `payload` and `payload` is
what §5 hashes, so a ceiling at `now` would give all twenty window documents
a new hash on every run: each a restatement of itself, at an ever higher
`revision`, carrying one of §8's four reasons for a figure that never moved.
Anchored to the archive, a run that ingested nothing computes the same
window, the same figures and the same hash.

A figure that is *about* the clock rather than about the window is not
covered by this and does not pretend to be: an open dispute's age (§6.7) is
`now - opened_at` and moves between two runs over the same archive, which is
a figure that really did change. An instance's `silent_for` is `now -
last_seen_at` and is the same shape, which is why the five `instances`
documents restate on every run alongside the five `disputes` ones — and why
those ten are re-signed whole even though most of what they carry is a
profile that has not moved in weeks.

#### 6.1.1 Scoped documents: one instance's orders, by currency

The grammar of §3 admits a `scope` on any window document. What is actually
published under one is the `orders` report, per instance:

```
orders:24h:i:<pubkey>
orders:7d:i:<pubkey>
orders:30d:i:<pubkey>
orders:90d:i:<pubkey>
orders:all:i:<pubkey>
```

One set per instance the archive knows — every instance the `instances`
document of the same snapshot lists, whether or not it traded, for the same
reason that document is published over an empty archive: a client cannot
tell a document that does not exist from a relay withholding one.

The payload is the report envelope of §6.1, and its metrics are the §6.1
activity block twice over:

- the instance's own figures, under the names the network-wide document
  uses — `orders.created`, `orders.completed`, `orders.open_now` and the
  rest, so one reader parses both scopes;
- the same block again per currency that instance traded in the window,
  with the code as a segment: `orders.ARS.created`, `orders.ARS.open_now`,
  `orders.USD.created`.

That cross of instance against currency is the one figure no other document
carries. `instances` and `compare` give one row per instance and `market`
gives the network's currency concentration; neither says that this instance
has thirteen ARS orders and two USD ones.

**The currency blocks partition the instance's orders.** An order names
exactly one currency and belongs to exactly one instance, so a client may
sum them — across currencies for the instance's total, across instances for
the network's — and nothing is counted twice. This is not true of payment
methods, which an order names several of and which are therefore attributed
rather than divided (SPEC §6.3); it is why currencies get blocks and methods
do not.

A currency the instance never traded in the window has no block at all,
rather than a block of zeros. Absence here is the §6.3 rule applied to a
dimension instead of to a bucket: what is not published is what nobody
published. The same holds for the network's blocks.

Neither reaches the series. `series:orders:*` has no column per currency and
will not gain one: a column per code per bucket is this same size argument
in the one document shape that already repeats a row per day.

**The network-wide `orders:<window>` carries the same currencies, four
figures deep.** Its blocks are `orders.<CODE>.created`, `.completed`,
`.canceled` and `.open_now` — no rates, no deltas, no `in_progress_now`.

The asymmetry is a size argument and nothing else. An instance's currencies
are the handful it lists in its kind 38385; the network's are every code any
instance has ever published, and that list has no ceiling while §9.1 does.
At roughly eighty-five bytes a metric, nine figures over a hundred codes is
some seventy-six kilobytes — past the default limit, in the largest document
of the snapshot, where the failure mode is a run that publishes nothing at
all. Four figures is a quarter of that.

Which four is not arbitrary either: they are the figures that *sum*. A
client may add a currency's blocks across instances and get the network's,
or compare them against the whole-network row above. Rates do not sum — a
completion rate is not the sum of completion rates — and `completion_rate`
is anyway `completed / (completed + canceled)`, which a client derives from
two of the four. What is left is published per instance for whoever wants it
there.

**Only `orders` is scoped.** The full cross product of report × window ×
instance is forty documents per instance for figures that are, for the other
seven reports, already published one row per instance in `compare` and
`instances`. §13.4 is the standing question about widening this; the answer
today is that it is not widened.

### 6.2 Series partitions

The flat form repeats the envelope of every metric on every row, which for
365 days is mostly overhead. Series partitions are columnar:

```json
{
  "schema_version": 1,
  "snapshot_id": "01J8Z…",
  "generated_at": "2026-08-27T03:05:00Z",
  "revision": 2,
  "restated_at": "2026-08-27T03:05:00Z",
  "restated_because": "backfill",
  "payload": {
    "period": { "from": "2026-01-01T00:00:00Z", "until": "2026-02-01T00:00:00Z" },
    "resolution": "daily",
    "columns": [
      { "name": "date",        "unit": "date" },
      { "name": "created",     "kind": "observed", "unit": "count" },
      { "name": "completed",   "kind": "observed", "unit": "count" },
      { "name": "volume_sats", "kind": "observed", "unit": "sats" },
      { "name": "volume_usd",  "kind": "inferred", "unit": "usd",
        "error": "rate snapshot at or before success_at; see SPEC §5" }
    ],
    "rows": [
      ["2026-01-01", 12, 7, 1361000, 980.4],
      ["2026-01-02", 0, 0, 0, null]
    ]
  }
}
```

`kind` and `error` are declared once per column rather than once per cell, so
the observed/inferred distinction survives compaction intact. `rows` is
ordered ascending by bucket, one entry per bucket in the period, none
skipped.

### 6.3 Absence

`null` means absence and never zero, throughout, matching the `—` of the
tables:

- A bucket with no activity has real zeros for counts and `null` for rates.
- A bucket **outside `coverage`** has `null` for every column, including
  counts. A relay keeps orders for about a fortnight, so a series reaching
  back before the first backfill would otherwise draw a flat line at zero
  across a period when the network was trading — the single most misleading
  output this system could produce.
- A partition entirely outside coverage is not published at all, and is
  absent from the index.

## 7. Atomicity

A snapshot is a few dozen documents and cannot be one event, so publication
is not atomic. A client that requests `summary:30d` and `volume:30d` while
the daemon is mid-publication can receive one from each of two snapshots and
render two figures that do not reconcile.

The index is the authority on what a coherent set is. It names every
document with the `hash` of the payload that belongs to the current snapshot,
so a conformant client:

1. Reads the index, takes its `snapshot_id` and its `documents` list.
2. Renders a document only when the SHA-256 of its `payload` equals the
   `hash` the index names for that `d`.
3. On a mismatch, marks that panel as updating and waits for the replacement
   — it MUST NOT mix snapshots silently.

The check is on the hash and **not** on the `s` tag, which would be the
obvious rule and is the wrong one. A document whose figures did not change is
not republished (§8), so it still carries the `snapshot_id` of the run that
last computed it — an older one, by design. Demanding an `s` match would
reject exactly the documents that are most certainly current: unchanged ones
are coherent with every snapshot since, which is what unchanged means.

`snapshot_id` is monotonic and unique per publication run, and the `s` tag
carries it so a client can ask a relay for a whole run in one filter. It is a
fetch hint and a provenance record — *which run last computed this* — not the
coherence test.

The daemon publishes the index **last**, so an index that names a set of
hashes implies the documents bearing them are already on the relay.

## 8. Restatement

A closed month is not immutable in practice. A later backfill discovers older
events, a `rebuild` recomputes projections, a dev fee arrives late. Published
partitions will change.

That is allowed, and it MUST NOT be silent:

- `revision` starts at 1 and increments on every change of `payload` — of
  the figures. A new `snapshot_id` or `generated_at` is a new run, not a new
  revision, which is why the hash of §5 covers the payload alone.
- `restated_at` and `restated_because` (`backfill` / `rebuild` /
  `schema` / `correction`) accompany any revision above 1.
- The index carries the same fields, so a client detects a restatement
  without fetching the partition.
- The reason is read off the archive, not off a flag. `publish` is not
  told whether a `backfill` or a `rebuild` ran before it, but
  `publication_runs` records the schema each run published under and the
  extent and size of the archive it read, which tells the three inferable
  reasons apart: a changed `schema_version` is `schema`; an archive that
  reaches further back, or holds more events than the last run saw, is
  `backfill`; the same events underneath moved figures is `rebuild`.
  `correction` is never inferred — nothing in the archive distinguishes
  it from a rebuild.
- Publication history lives in the archive (`published_documents`,
  `publication_runs`), not on a relay. It is not derivable from the Mostro
  events, and reading the last index back off a relay would work until the
  day a relay pruned it — after which every revision would silently reset
  to 1, which is exactly the claim this section exists to make
  trustworthy. What §9.3 refuses to keep is a cache of *signed events*;
  these tables hold none.
- A client that has cached a partition and sees a higher revision SHOULD
  surface that the figure changed, not just swap it.

During an ordinary publication run, a document whose payload hashes to what
is already published is not re-signed and not sent: nothing about the answer
changed, and a relay does not need a second copy of it. The index still lists
it, with its existing hash, revision and `updated_at`, because "unchanged" is
one of the things the index exists to say.

The index is the exception to this rule, and §5 says why: nothing hashes it,
it has no `payload` to compare and no `revision` to count, and naming the
current snapshot is its whole job. A run over an archive that has not moved
therefore re-sends no document — and the index anyway.

The exception is §9.3. `--republish` puts documents on a relay that does not
have them, so "the relay already has this" is precisely the assumption it
exists to distrust; it regenerates and signs unconditionally.

## 9. Size

### 9.1 Limits

A relay advertises `limitation.max_content_length` in its NIP-11 document.
The daemon MUST read it at startup for each configured relay and MUST
validate every document against the smallest of them and against its own
configured ceiling (`[publish].max_content_bytes`, default 64 KiB,
conservative on purpose).

A document that exceeds the ceiling is a **startup or publish error naming
the document**, never a silent relay rejection. This mirrors the existing
rule that a bad configuration value is an error naming the key.

### 9.2 The resolution ladder

Partitioning bounds the size of each document; the ladder bounds how many a
client fetches. Daily buckets are grouped by month, weekly by month, monthly
by year, so:

| Range requested | Resolution | Partitions |
| --- | --- | --- |
| < 90 days | daily | ≤ 4 |
| < 2 years | weekly | ≤ 24 |
| ≥ 2 years | monthly | ≤ 10 for a decade |

A ten-year range costs ten events. The client picks the resolution from the
requested span and from `resolutions` in the index; the daemon publishes all
three for the periods it covers.

### 9.3 Republication

`bestiario publish --republish [--from <bucket>] [--until <bucket>]`
regenerates and publishes every document for a range from the archive,
independent of what any relay holds. This is the recovery path for a pruned
relay, a new relay, or a schema migration, and it is what makes §1.1 true
rather than aspirational.

It therefore **overrides the skip of §8**, which would otherwise defeat it:
the documents a pruned relay is missing are overwhelmingly the ones whose
figures have not changed in months, and a run that skipped them would send
the recovering relay nothing. Every document in the range is regenerated,
signed and sent, whatever its hash.

`--from` / `--until` select **partitions**, which is the only reading of "a
range" a partitioned format allows; a window document is relative to the
archive's ceiling and covers no fixed span, so no range names one. Whatever
an ordinary run would have sent is still sent alongside: withholding a
*changed* document because it fell outside the range would publish an index
naming a hash no relay holds, which is the one thing §7 forbids.

Signed afresh rather than replayed from a store of past events: §1.1 puts the
whole truth in the archive and keeps no state beside it, and a cache of
signed events would be exactly such state — one that can disagree with the
archive it was derived from. Re-signing an unchanged payload is not a
restatement and does not touch `revision`; it is one publication run like any
other, with its own `snapshot_id`, and the index goes last as always.

Series partitions MUST NOT carry a NIP-40 `expiration` tag. Hot window
documents MAY.

### 9.4 Write volume

§9.1 bounds how large a document may be; nothing so far bounds how *many* a
run sends. A snapshot is now a burst: eight reports over five windows, the
series partitions of every covered month and year, five documents per
instance (§6.1.1), and the index. A dozen instances puts it above a hundred
events, sent back to back over one connection.

Relays meter writes. A limit in the tens of events per minute is ordinary,
and a relay that hits one does not usually explain itself: it stops
answering `OK`, and the documents it stopped answering for are the tail of
the run. That failure is loud here rather than silent — a document no relay
accepted fails the run and the index naming it is not sent (§7) — which is
the right behaviour and not a comfortable one, because the run that fails
is every run.

Two things follow for an operator. Publish to relays whose write limits you
know, ideally one you run (§13.2). And note that the skip of §8 does most
of the work in the steady state: a run over an archive that gained a few
orders re-sends the documents whose figures moved and the index, not the
hundred. It is the first run against a fresh relay, and any `--republish`
(§9.3), that sends everything at once.

## 10. Client algorithm

Normative for `mostro.world` and recommended for any other consumer:

1. Fetch `index` for the known publisher pubkey. Verify the signature; abort
   on failure.
2. Take `snapshot_id`, `coverage`, `resolutions`.
3. For the visible panels, request the hot documents by `d`; subscribe for
   live replacement.
4. For a requested range: choose a resolution (§9.2), enumerate the
   partitions covering it, drop those with a cached `hash` matching the
   index, request the rest in one `REQ`.
5. Verify every event's signature and that the SHA-256 of its `payload`
   matches the `hash` the index names for that `d` (§7). Drop and re-request
   otherwise. Do not test the `s` tag for equality with the index's
   `snapshot_id`: an unchanged document legitimately carries an older one.
6. Render `inferred` figures visually distinct from `observed` ones, with the
   column's `error` text reachable. Render `null` as absence, never as zero.
7. Show the age of the data, computed from `created_at`, and warn beyond a
   configured threshold.

Closed partitions with an unchanged hash are immutable and MAY be cached
indefinitely.

## 11. Tags

| Tag | Indexed | Value |
| --- | --- | --- |
| `d` | yes | Document address, §3 |
| `s` | yes | `snapshot_id` of the run that last computed the payload — a fetch hint, not the coherence test, §7 |
| `t` | yes | `bestiario` — discovery |
| `alt` | no | Human-readable description, NIP-31 |
| `resolution` | no | `daily` / `weekly` / `monthly`, series only |
| `period` | no | `<from>` `<until>`, RFC3339, series only |
| `revision` | no | Integer, §8 |
| `schema_version` | no | Integer |

Only single-letter tags are relay-indexed; the rest are for clients that hold
the event.

## 12. CLI

```
bestiario publish [--dry-run] [--out <dir>] [--republish] [--from …] [--until …]
```

- Computes an entire snapshot in one pass over the archive, so all documents
  share a consistent view. It does not shell out to the report commands.
- `--dry-run` prints what would be published, with sizes and hashes, and
  signs nothing.
- `--out` also writes the documents as files, for the static fallback
  snapshot the site serves before the relay connection is live.
- Signing key from the environment, named by `[publish].nsec` as
  `"env:BESTIARIO_PUBLISH_NSEC"`; never from a flag, and never written
  into the configuration file. A flag is readable in `ps` and lands in a
  shell history; a configuration file is copied, committed and pasted into
  issues. `[publish].nsec` therefore parses into a type that holds a
  variable's *name*, so a literal key there is a load error rather than a
  working setup that leaks. The variable is read only by a run that is
  going to sign, not when the configuration loads, so a command that
  publishes nothing — `--dry-run` included — does not require it.
- Publishes to `[publish].relays`, which defaults to `[nostr].relays` but is
  configured separately: reading and writing are different trust decisions.

Aggregation stays in `crates/stats`, with no I/O, as everywhere else.

## 13. Open questions

1. **Upstream registration of 30666** — when to open the PR against
   `nostr-protocol/nips`. Proposal: after the first production snapshot, so
   the registered description matches something that exists.
2. **Relay retention** — whether MostroP2P should run a relay that guarantees
   the historical partitions, given §1.1. Cheap on the same VPS; the
   alternative is accepting that history may need republishing.
3. **Auth** — relays requiring NIP-42 complicate every client for no benefit
   here. Proposal: the default relay set excludes them, and the daemon warns
   when a configured relay advertises it.
4. **Per-instance documents beyond `orders`** — the window half of this is
   settled: `orders:<window>:i:<pubkey>` is published for every instance
   (§6.1.1), because the cross of instance against currency exists nowhere
   else. The rest is still open. Per-instance *series* remain unpublished —
   the full cross product (report × resolution × bucket × instance) is a
   large number of documents; the proposal, if they are ever wanted, is
   `orders` and `volume` at monthly resolution only. Scoping the other six
   window reports is likewise unpublished: `compare` and `instances` already
   carry their per-instance figures a row at a time.
5. **Signature verification in-browser** — which library, and whether the
   site fails closed (render nothing) or degrades to the static snapshot with
   a visible warning. Proposal: fail closed for signature failure, degrade for
   connection failure.
