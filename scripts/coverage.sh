#!/usr/bin/env bash
#
# The coverage gates of docs/SPEC.md §12, from one measurement.
#
# Two numbers, because averaging them would hide the one that matters.
# `crates/stats` and `src/ingest/parse` are plain functions over plain
# data: a line no test executed there is a case nobody thought about, not
# a case that is awkward to reach, so nothing in them may go unexecuted.
# Everything else is gated at 95%; the residue is I/O failure handling — a
# relay dropping mid-subscription, a full disk — where the test costs more
# than the line is worth, and that judgement is recorded here rather than
# left as an unexplained shortfall.
#
# Both gates read the same JSON report, so they cannot disagree about what
# was measured. `cargo llvm-cov report` is deliberately not used: it takes
# no `--workspace`, and scoped to one package it reports a total that
# leaves the other crate's files out — which passes a per-layer gate by
# never looking at the layer it is named after.
#
# Usage: scripts/coverage.sh [--clean] [--open]

set -euo pipefail

# A percentage is printed and compared here; under a locale whose decimal
# separator is a comma, `printf %.2f` refuses the number jq produced and
# the gate fails for a reason that has nothing to do with coverage.
export LC_ALL=C

OVERALL_MIN=95
PURE_LAYERS=("crates/stats/" "src/ingest/parse/")

cd "$(dirname "$0")/.."

# Stale profiles from an earlier build would credit lines this tree no
# longer has. CI starts from a checkout; a developer running this against
# a changed tree wants `--clean`.
for argument in "$@"; do
  [ "$argument" = "--clean" ] && cargo llvm-cov clean --workspace
done

report=$(mktemp)
listing=$(mktemp)
trap 'rm -f "$report" "$listing"' EXIT

echo "Measuring the workspace…"
cargo llvm-cov --workspace --all-features --json --output-path "$report" >/dev/null

# The same suite again, for llvm's own list of uncovered lines. A second
# run is the price of gate 2 being checked by something other than the code
# that implements it: this gate has already passed three times while seeing
# nothing, and no single clever implementation catches its own blindness.
echo "Measuring again for llvm's own reading…"
cargo llvm-cov --workspace --all-features --summary-only --show-missing-lines > "$listing"

# The report is fed on stdin throughout: given a filename positionally, jq
# with `--args` reads stdin anyway, which is a hang at best and an empty
# answer — a gate that passes without looking — at worst.
overall=$(jq -r '.data[0].totals.lines.percent' < "$report")
files=$(jq -r '.data[0].files | length' < "$report")
printf '\nMeasured %s files, %.2f%% of lines overall.\n' "$files" "$overall"

status=0

printf '\nGate 1: the workspace at %s%% of lines\n' "$OVERALL_MIN"
if jq -e --argjson min "$OVERALL_MIN" '.data[0].totals.lines.percent >= $min' < "$report" >/dev/null; then
  printf '  %.2f%% — passes\n' "$overall"
else
  printf '  %.2f%% — below %s%%\n' "$overall" "$OVERALL_MIN"
  status=1
fi

printf '\nGate 2: every line of %s executed\n' "${PURE_LAYERS[*]}"
python3 scripts/uncovered.py "$report" --llvm "$listing" "${PURE_LAYERS[@]}" || status=1

# Before exiting, not after: a failing gate is exactly when the annotated
# report is worth opening, and `--open` that only ever repeated the failure
# would send a reader to a page they cannot reach.
for argument in "$@"; do
  [ "$argument" = "--open" ] && cargo llvm-cov report --html --open
done

if [ "$status" -ne 0 ]; then
  echo
  echo "Run scripts/coverage.sh --open to see the report."
fi

exit "$status"
