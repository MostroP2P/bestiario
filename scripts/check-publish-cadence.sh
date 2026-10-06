#!/usr/bin/env bash
# Tests for the publication loop of deploy/replicated.sh, with a stand-in
# `bestiario` on the PATH. What is checked is the cadence: the interval is
# counted from the start of one publication to the start of the next, so a
# publication that runs long delays the next by its overrun and no more.
# Counted from the end instead, every second a publication takes is added to
# the interval — which is how a five-minute cadence became a twenty-minute
# one in production, while each run spent sixteen minutes on a silent relay.
set -euo pipefail

wrapper=$(cd "$(dirname "$0")/.." && pwd)/deploy/replicated.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

failures=0
check() {
	local what=$1 expected=$2 actual=$3
	if [ "$expected" = "$actual" ]; then
		echo "ok   $what"
	else
		echo "FAIL $what" >&2
		echo "  expected: $expected" >&2
		echo "  actual:   $actual" >&2
		failures=$((failures + 1))
	fi
}

# The stand-in: `publish` records when it starts and then takes three
# seconds; anything else is the daemon, which lives long enough for a few
# publications and then exits, taking the wrapper with it.
mkdir "$tmp/bin"
cat >"$tmp/bin/bestiario" <<EOF
#!/bin/sh
if [ "\$1" = publish ]; then
	date +%s >>"$tmp/starts"
	sleep 3
else
	sleep 11
fi
EOF
chmod +x "$tmp/bin/bestiario"

# An interval of two seconds and publications of three: from start to start,
# each publication begins as soon as the previous one ends, three seconds
# apart. Counted from the end, they would be five seconds apart.
env -u LITESTREAM_BUCKET PATH="$tmp/bin:$PATH" BESTIARIO_PUBLISH_EVERY=2 \
	sh "$wrapper" sync >/dev/null 2>&1 || true
# Without a bucket the wrapper execs the daemon, and the publication loop
# outlives it: stop it here rather than leave it publishing into a directory
# about to be removed.
pkill -TERM -f "$wrapper" || true

starts=$(cat "$tmp/starts" 2>/dev/null || true)
check "the daemon's lifetime holds at least three publications" "yes" \
	"$([ "$(wc -l <<<"$starts")" -ge 3 ] && echo yes || echo no)"

late=0
previous=""
while read -r start; do
	if [ -n "$previous" ] && [ $((start - previous)) -gt 4 ]; then
		late=$((late + 1))
	fi
	previous=$start
done <<<"$starts"
check "no publication waits out the interval after the previous one ends" "0" "$late"

if [ "$failures" -ne 0 ]; then
	echo "$failures check(s) failed" >&2
	exit 1
fi
echo "publish cadence: all checks passed"
