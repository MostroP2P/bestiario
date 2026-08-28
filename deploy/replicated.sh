#!/bin/sh
#
# Runs bestiario with its database replicated to object storage.
#
# App Platform gives every container a clean filesystem, so without this the
# index is rebuilt from the relays on each deploy, restart and rescale — and
# only as far back as the relays still hold. litestream turns that filesystem
# into a cache of a bucket: it restores the database before the daemon starts
# and streams every subsequent write out as it happens.
#
# Usage mirrors the binary: `bestiario-replicated sync`, `… backfill`.
#
# Installed as /usr/local/bin/bestiario-replicated, and the image's ENTRYPOINT,
# so that one image has one contract. With no bucket configured it execs
# bestiario unchanged and `docker run … summary` behaves exactly as before;
# replication is what a bucket in the environment adds, not a different image
# and not a different command.

set -eu

: "${BESTIARIO_DB_PATH:=/data/bestiario.db}"
export BESTIARIO_DB_PATH

# Without a bucket there is nothing to replicate to, and refusing to start
# would make the image unusable for every local `docker run`. Run the daemon
# plainly and say so once, rather than failing or — worse — pretending to
# replicate.
# `sync` subscribes to the relays from roughly now and does not walk their
# history, so a deployment that only syncs holds the last few days and presents
# them as the network.
#
# The sequence cannot be written as `backfill && sync`. Neither App Platform's
# run_command nor litestream's -exec is guaranteed to reach a shell — the first
# exits non-zero before the daemon starts, and the second runs the backfill and
# then shuts down without ever reaching the sync. So the two are sequenced here,
# as separate processes, and nothing downstream is ever handed a shell operator.
backfill_first=""
if [ "${BESTIARIO_BACKFILL_FIRST:-}" = "true" ]; then
    backfill_first="yes"
fi

if [ -z "${LITESTREAM_BUCKET:-}" ]; then
    echo "bestiario-replicated: LITESTREAM_BUCKET is unset, running without replication" >&2
    [ -z "$backfill_first" ] || bestiario backfill
    exec bestiario "$@"
fi

: "${LITESTREAM_PATH:=bestiario}"
: "${LITESTREAM_REGION:=nyc3}"
: "${LITESTREAM_ENDPOINT:=${LITESTREAM_REGION}.digitaloceanspaces.com}"
export LITESTREAM_PATH LITESTREAM_REGION LITESTREAM_ENDPOINT

# `-restore-if-db-not-exists` covers both the first boot, when the bucket is
# empty and there is nothing to restore, and every boot after it, when there
# is. `-exec` ties the two processes together: litestream exits when bestiario
# does, and forwards the signal the other way, so App Platform's SIGTERM
# reaches the daemon and the final pages reach the bucket before the container
# goes away.
#
# One writer, and one only. litestream's lock lives inside the SQLite file, so
# it cannot see a second container holding the same bucket prefix — two of them
# would interleave writes into one replica and corrupt it. Keep instance_count
# at 1, and give any second deployment its own LITESTREAM_PATH.
# litestream's -exec takes one string and hands it to a shell, so the argument
# boundaries have to survive being written down. `$*` would not: it joins on
# spaces, and `publish --out "/data/my snapshots"` would reach bestiario as two
# arguments instead of one. Quote each argument instead, closing and reopening
# the quoting around any embedded single quote — '\'' is the only form a POSIX
# shell reads back as the character itself.
command="bestiario"
for arg in "$@"; do
    command="$command '$(printf '%s' "$arg" | sed "s/'/'\\\\''/g")'"
done

# Restore explicitly, rather than leaving it to `replicate
# -restore-if-db-not-exists`, so the backfill below sees the index that is
# already in the bucket and walks the relays for what is missing from it
# rather than for all of it.
litestream restore -if-db-not-exists -if-replica-exists "$BESTIARIO_DB_PATH"

# The backfill runs before replication starts, so its writes are not streamed
# as they happen; the snapshot litestream takes when it starts carries them
# instead. A container that dies mid-backfill therefore loses that pass and
# redoes it on the next start, which is exactly what an idempotent backfill is
# for.
[ -z "$backfill_first" ] || bestiario backfill

exec litestream replicate -exec "$command"
