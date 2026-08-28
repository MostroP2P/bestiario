# Deploying bestiario

How to run the daemon as a container, and specifically as a DigitalOcean
App Platform app. `README.md` covers running it from a checkout; this
document covers running it somewhere nobody logs into.

Three files carry the deployment, and all three are versioned:

| File            | What it is                                         |
| --------------- | -------------------------------------------------- |
| `Dockerfile`    | The image: a builder stage and a small runtime one  |
| `.dockerignore` | What never enters the build context                 |
| `.do/app.yaml`  | The App Platform spec, applied with `doctl`         |
| `deploy/replicated.sh` | The entrypoint: restore, then run, then stream |
| `deploy/litestream.yml` | Which database goes to which bucket          |

## Configuration without a file

The image ships no `settings.toml`. Every setting is supplied as a
`BESTIARIO__*` environment variable, and the mapping is mechanical: two
underscores separate the section from the key, so `[database].url` is
`BESTIARIO__DATABASE__URL`. Lists are one variable with comma-separated
values:

```
BESTIARIO__NOSTR__RELAYS="wss://relay.mostro.network,wss://nos.lol"
BESTIARIO__INDEXER__INSTANCES="82fa8cb9…,0f2a5b1c…"
BESTIARIO__INDEXER__NETWORKS="mainnet,testnet"
```

A missing file is tolerated **only** at the default path. Naming a path
explicitly and getting it wrong is still an error: a typo in `-c` must not
quietly become "index with defaults". The tolerance exists for exactly one
case — the container that was never given a file.

Tolerance is not a fallback to defaults. The settings without a defensible
default are still checked at startup, so a worker deployed with no relays
configured fails immediately with `[nostr].relays is empty` rather than
running forever and reporting zeros.

`[assumptions.dev_fee_percentage]` is the one setting the environment cannot
express, because it is a map keyed by pubkey. A deployment that needs
per-instance overrides has to mount a `settings.toml`; the environment layer
still applies on top of it.

## The signing key

Two variables, and telling them apart matters:

- `BESTIARIO__PUBLISH__NSEC` is **configuration**. Its value is
  `env:BESTIARIO_PUBLISH_NSEC` — the *name* of the variable holding the key.
  Writing an actual `nsec1…` here is refused at startup.
- `BESTIARIO_PUBLISH_NSEC` is the **key**. One underscore, so it falls
  outside the `BESTIARIO__` prefix and is never read as a setting.

Only the second is a secret, and it is marked `type: SECRET` in
`.do/app.yaml` — as are the two Spaces credentials below. The indirection is
what lets the spec be committed: it names where the key lives without ever
containing it.

The key is read when a run actually signs. A worker that only runs `sync`
neither needs the variable nor fails without it.

## Running the image

```sh
docker build -t bestiario .

docker run --rm \
  -e BESTIARIO__NOSTR__RELAYS="wss://relay.mostro.network" \
  -e BESTIARIO__INDEXER__INSTANCES="82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390" \
  -v bestiario-data:/data \
  bestiario sync
```

Any subcommand works as the argument: `backfill`, `summary --json`,
`publish --dry-run`. Without `LITESTREAM_BUCKET` in the environment nothing
is replicated and the container keeps its database in the volume, which is
what a local run wants.

## App Platform

```sh
doctl apps create --spec .do/app.yaml --project-id <MOSTRO_PROJECT_ID>
doctl apps update <APP_ID> --spec .do/app.yaml
```

`--project-id` puts the app in the Mostro project alongside the rest of the
network's infrastructure; without it DigitalOcean files it under the
account's default project. The spec has no field for this — it is a
create-time flag, so it is easy to forget and awkward to correct later. The
Spaces bucket belongs in the same project:
`doctl projects resources assign <ID> --resource do:space:<bucket>`.

Before the first `create`, DigitalOcean's GitHub app has to be authorised
for the organisation, or the API answers `GitHub user does not have access
to MostroP2P/bestiario`. That is an interactive grant in the dashboard —
**Apps → Create → GitHub → Manage Access** — with no `doctl` equivalent.

Edit the spec, not the dashboard. A change to which instances are indexed is
a change to what the published statistics mean, and it should go through
review like any other.

Three properties of the platform shape the spec:

**No Rust buildpack.** App Platform builds Go, Node, Python, PHP, Ruby and
static sites natively; Rust is not on the list. The `Dockerfile` is not an
optimisation, it is the only way in.

**No HTTP surface, so a worker.** bestiario listens on nothing. That makes it
a `worker` component rather than a `service`: no port, no health check, no
public URL — and workers require a paid plan.

**Backfill, then sync.** `sync` subscribes to the relays from roughly the
moment it starts; it does not walk their history. A worker running only
`sync` therefore reports on the last few days and presents that as the
network — the quiet failure `docs/SPEC.md` warns about, since nothing errors
and every number looks plausible. The spec's `run_command` runs `backfill`
first, on every start. That repeats work, but the backfill is idempotent and
costs a couple of minutes, and it means the index repairs itself if the
replica is ever lost or truncated.

**No scheduler, and no publication job.** App Platform `jobs` run
`PRE_DEPLOY`, `POST_DEPLOY` or `FAILED_DEPLOY`, and nowhere else. There is no
cron. `.do/app.yaml` defines no job all the same, because the only shape
available would publish the wrong thing or corrupt the replica:

- a job is a component of its own, with its own container and its own
  ephemeral `/data`. Given no litestream configuration it starts from an
  empty database, and `bestiario publish` then replaces the addressable
  documents with statistics computed from nothing;
- given the *same* litestream configuration as the indexer, it becomes the
  second writer against one bucket prefix — the thing "One writer, and one
  only" below says must not happen.

Publication has to run where the archive is, and the ways there are:

- a `--every` interval inside the daemon, so one worker both syncs and
  publishes against the same file — the option that keeps the deployment a
  single component, and the one to reach for first;
- a separate component with a `LITESTREAM_PATH` of its own, which makes it a
  publisher of its own index rather than of the indexer's;
- Postgres, per "Postgres, still" below, after which more than one component
  may touch the same data safely.

## The ephemeral filesystem, and the bucket that outlives it

App Platform components have no persistent volumes. Every deploy, restart
and rescale starts from a clean filesystem, so `/data/bestiario.db` is gone
and `create_if_missing` makes a fresh, empty one. Left alone, the index is
rebuilt from whatever the relays still hold — which is not what was indexed,
since relays expire events and `backfill_from` only reaches as far back as
they kept.

The deployment therefore replicates the database to a DigitalOcean Spaces
bucket with [litestream](https://litestream.io), which turns the container's
filesystem into a cache of that bucket: the database is restored before the
daemon starts and every subsequent write is streamed out as it happens.

`deploy/replicated.sh` is the whole of it, and it is the image's
`ENTRYPOINT`, so one image has one contract. With no `LITESTREAM_BUCKET` in
the environment it execs bestiario unchanged — `docker run … summary`
behaves exactly as it did before replication existed. With a bucket, the
same invocation replicates.

### Configuration

| Variable | What it is |
| --- | --- |
| `LITESTREAM_BUCKET` | The Spaces bucket. Unset means "do not replicate". |
| `LITESTREAM_PATH` | A prefix *inside* the bucket, not a filename. |
| `LITESTREAM_REGION` | `nyc3`, and the default for the endpoint. |
| `LITESTREAM_ENDPOINT` | `nyc3.digitaloceanspaces.com`. |
| `LITESTREAM_ACCESS_KEY_ID` | Spaces key, scoped `readwrite` to the one bucket. Stored as a `SECRET`. |
| `LITESTREAM_SECRET_ACCESS_KEY` | Its secret, likewise a `SECRET`. |
| `BESTIARIO_DB_PATH` | The file litestream replicates. |

`BESTIARIO_DB_PATH` and `BESTIARIO__DATABASE__URL` must name the same file.
They are set together in the Dockerfile for that reason: bestiario is told a
SQLite URL and litestream a filesystem path, and replicating a different file
than the daemon writes would back up an empty database without ever failing.

Create the key scoped to the single bucket rather than account-wide:

```sh
doctl spaces keys create bestiario-litestream \
  --grants 'bucket=mostro-bestiario-index;permission=readwrite'
```

### One writer, and one only

litestream's lock lives inside the SQLite file, so it cannot see a second
container replicating the same bucket prefix. Two of them would interleave
writes into one replica and corrupt it. Three consequences, none of them
optional:

- `instance_count` stays at **1**. It is not a scaling knob.
- A second deployment gets its own `LITESTREAM_PATH`.
- A `publish` job sharing the prefix would be that second writer. Publish
  from the same process as `sync`, or from a prefix of its own.

The honest caveat: App Platform may briefly overlap the old and new
containers during a deploy, and nothing here prevents that window.

If a replica is damaged, **copy the prefix aside before touching it** and try
to restore from it — `litestream restore -o /tmp/check.db` names a different
output file and leaves the replica alone, so a partial recovery is still on
the table. Only once that is exhausted, empty the prefix and re-run
`backfill`.

That last step is a real loss, not a free reset. bestiario never overwrites
history (`docs/SPEC.md` §5), so nothing is corrupted silently — but a
backfill reaches only as far back as the relays still hold, and they expire
events. Whatever the index had recorded from before that horizon does not
come back. The replica *is* the durable copy; the relays are not a backup of
it. A deployment that cannot accept that risk wants Postgres, not a second
replica.

### Postgres, still

Replication makes the index durable; it does not make SQLite a networked
database. A deployment that wants concurrent readers, or more than one
component touching the data, wants Postgres: a `postgres` feature on `sqlx`
(`Cargo.toml` compiles only `sqlite` today), a second `Migrator`, and
relaxing the `sqlite:` check in `src/db/mod.rs`.
