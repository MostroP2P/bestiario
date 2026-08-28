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

# Publication, on an interval, beside the daemon.
#
# App Platform has no scheduler and a POST_DEPLOY job would be a second
# container with its own empty /data — publishing statistics computed from
# nothing, or replicating over this one's bucket prefix. So the interval lives
# here, in the process that already has the index.
#
# `publish` reads the archive to compute the snapshot and writes once, at the
# end: a single transaction recording the run and the documents it sent. So it
# is a second writer, not a second reader — which is still ordinary for one
# SQLite file in WAL mode, the two being serialised by the write lock and the
# busy timeout the pool sets, and which litestream replicates like any other
# write to the file it watches.
#
# BESTIARIO_PUBLISH_EVERY is a `sleep` duration, so `21600`, `6h` and `90m` all
# work. Unset means never.
publish_every() {
    # A TERM from the shutdown below means "no more publications". It ends the
    # wait for the next interval at once, but a publication already in flight
    # is left to finish: the shell defers a trap until the foreground command
    # returns, and that is exactly the behaviour wanted here. `publish` sends
    # to the relays first and records what it sent afterwards, so a run killed
    # between the two leaves documents on the relays that the archive does not
    # know it published.
    stopping=""
    trap 'stopping=yes' TERM

    # The first publication waits out a whole interval rather than firing at
    # startup. A container that is crash-looping restarts every few seconds,
    # and publishing on each start would sign and broadcast a document storm
    # to the relays — the one failure here that other people would notice.
    while [ -z "$stopping" ]; do
        # Slept in the background and waited for, rather than slept in the
        # foreground: `wait` returns the moment the signal arrives, where
        # `sleep 6h` as a foreground command would hold the trap — and the
        # container's shutdown — for up to six hours.
        sleep "$BESTIARIO_PUBLISH_EVERY" &
        sleep_pid=$!
        wait "$sleep_pid" || true
        if [ -n "$stopping" ]; then
            kill "$sleep_pid" 2>/dev/null || true
            break
        fi

        if bestiario publish; then
            echo "bestiario-replicated: published" >&2
        else
            # A failed publication must not end the loop. Relays refuse
            # connections, keys expire, and the next interval is a better
            # answer than a worker that has quietly stopped publishing.
            echo "bestiario-replicated: publish failed, next attempt in ${BESTIARIO_PUBLISH_EVERY}" >&2
        fi
    done
}

# A cadence is one environment variable away from being a document storm.
# `sleep 0` returns immediately, so `0` — or `0s`, or `0.0` — would publish in
# a tight loop: with a key, signing and broadcasting a snapshot as fast as the
# relays accept it; without one, burning a core and filling the log. Anything
# `sleep` cannot read at all is the same typo caught one step earlier.
#
# The check is a positive number with an optional unit, which is what the
# documented values are. `sleep` on some systems takes more (`1h 30m`, `1d`);
# refusing those costs an operator a unit conversion and is worth it here.
publish_every_is_valid() {
    every="${1%[smhd]}"
    case "$every" in
        # Not a number: empty, a stray character, or two decimal points.
        '' | *[!0-9.]* | *.*.*) return 1 ;;
    esac
    # Zero in every spelling — `0`, `00`, `0.0`, `.0` — has no non-zero digit.
    case "$every" in
        *[1-9]*) return 0 ;;
    esac
    return 1
}

# Refused at startup, before the daemon is up, rather than left to be
# discovered from the log of a worker that is publishing every few
# milliseconds. It costs a crash loop, which the deployment already alerts on
# and which names the variable and the value in its first line.
if [ -n "${BESTIARIO_PUBLISH_EVERY:-}" ] && ! publish_every_is_valid "$BESTIARIO_PUBLISH_EVERY"; then
    echo "bestiario-replicated: BESTIARIO_PUBLISH_EVERY is '${BESTIARIO_PUBLISH_EVERY}', which is not a positive sleep duration — use a value like 6h, 90m or 21600, or unset it to publish never" >&2
    exit 1
fi

# Announced at startup, before anything can go wrong, so the logs say what the
# worker intends to do rather than leaving it to be inferred from silence.
if [ -n "${BESTIARIO_PUBLISH_EVERY:-}" ]; then
    echo "bestiario-replicated: publishing every ${BESTIARIO_PUBLISH_EVERY}" >&2
else
    echo "bestiario-replicated: BESTIARIO_PUBLISH_EVERY is unset, not publishing" >&2
fi

if [ -z "${LITESTREAM_BUCKET:-}" ]; then
    echo "bestiario-replicated: LITESTREAM_BUCKET is unset, running without replication" >&2
    [ -z "$backfill_first" ] || bestiario backfill
    [ -z "${BESTIARIO_PUBLISH_EVERY:-}" ] || publish_every &
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

# Started after the backfill, never during it: publishing halfway through the
# history walk would sign a snapshot of a partial index and present it as the
# network.
publisher=""
if [ -n "${BESTIARIO_PUBLISH_EVERY:-}" ]; then
    publish_every &
    publisher=$!
fi

# Supervised rather than `exec`ed, which is what having a second process here
# costs. Under `exec` this shell would be replaced, App Platform's SIGTERM
# would reach litestream alone, and the publisher — a sibling nobody is
# waiting for — would be killed by the container going away, possibly in the
# middle of a run and possibly after litestream had already replicated for the
# last time. So the shell stays as pid 1 and shuts the two down in order.
litestream replicate -exec "$command" &
litestream=$!

# The order is the whole point: the publisher is stopped and waited for first,
# so that a publication in flight finishes and its record reaches the bucket,
# and only then is litestream asked to stop replicating. A run longer than the
# platform's grace period is still killed — the wait cannot buy more time than
# the container has — but it is no longer cut short by a shutdown that had
# nothing else left to do.
terminating=""
shutdown() {
    terminating="yes"
    if [ -n "$publisher" ]; then
        kill -TERM "$publisher" 2>/dev/null || true
        wait "$publisher" 2>/dev/null || true
    fi
    kill -TERM "$litestream" 2>/dev/null || true
}
trap shutdown TERM INT

status=0
wait "$litestream" || status=$?
if [ -n "$terminating" ]; then
    # A `wait` the signal interrupted returns 128+SIGTERM whatever litestream
    # then goes on to do, so waiting a second time is how the shell holds the
    # container open until the final pages have actually reached the bucket —
    # and litestream's status afterwards is not something this `wait` can
    # report anyway.
    wait "$litestream" 2>/dev/null || true
    # A shutdown that was asked for is not a failure. Exiting 143 here would
    # make an ordinary deploy indistinguishable from the crash the deployment
    # alerts on.
    status=0
fi

exit "$status"
