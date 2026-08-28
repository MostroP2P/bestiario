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

Only the second is a secret, and it is the only value in `.do/app.yaml`
marked `type: SECRET`. The indirection is what lets the spec be committed:
it names where the key lives without ever containing it.

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

The entrypoint is the binary, so any subcommand works as the argument:
`backfill`, `summary --json`, `publish --dry-run`.

## App Platform

```sh
doctl apps create --spec .do/app.yaml
doctl apps update <APP_ID> --spec .do/app.yaml
```

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

**No scheduler.** App Platform `jobs` run `PRE_DEPLOY`, `POST_DEPLOY` or
`FAILED_DEPLOY`, and nowhere else. There is no cron. The commented-out job in
`.do/app.yaml` publishes once per deploy, which is a deployment trigger and
not a publication schedule. Publishing on a real cadence needs one of:

- a `--every` interval inside the daemon, so one worker both syncs and
  publishes — the option that keeps the deployment a single component;
- a DigitalOcean Function on a schedule, or any external cron, invoking the
  app;
- a separate always-on worker whose command is a sleep loop around `publish`.

## The ephemeral filesystem

This is the part to read before treating the deployment as durable.

App Platform components have no persistent volumes. Every deploy, restart
and rescale starts from a clean filesystem, so `/data/bestiario.db` is gone
and `create_if_missing` makes a fresh, empty one. The index is then rebuilt
from whatever the relays still hold — which is not the same as what was
indexed, since relays expire events and `backfill_from` only reaches as far
back as they kept.

That is survivable for a deployment that redeploys rarely and only wants
recent statistics. It is not a durable index. The ways out, in order of how
much work they are:

1. **Postgres.** The honest fix. It needs a `postgres` feature on `sqlx`
   (`Cargo.toml` compiles only `sqlite` today), a second `Migrator`, and
   relaxing the `sqlite:` check in `src/db/mod.rs`. Tracked separately from
   this deployment work.
2. **Replicate the SQLite file** to Spaces, Litestream-style. Fewer code
   changes, more moving parts, and WAL replication has to be reasoned about
   rather than assumed.
3. **Accept the rebuild.** Viable while the relays hold enough history and
   the backfill is quick. Measure the backfill before choosing this.

Until one of those lands, treat the App Platform database as a cache of the
relays rather than as the record.
