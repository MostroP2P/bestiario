#!/usr/bin/env bash
#
# Enforces the invariant documented in src/stats/mod.rs and docs/SPEC.md §8:
# the stats layer performs no I/O, so every aggregation stays testable with a
# hand-built dataset and an HTTP API can reuse the layer unchanged.
#
# Line comments are stripped before matching, so the module docs are free to
# name the very crates they forbid.

set -euo pipefail

FORBIDDEN='sqlx|nostr_sdk|tokio|reqwest|std::fs|std::net|std::process'
status=0

while IFS= read -r file; do
    hits=$(sed -E 's;[[:space:]]*//.*$;;' "$file" | grep -nE "\b(${FORBIDDEN})\b" || true)
    [ -n "$hits" ] || continue
    status=1
    while IFS= read -r hit; do
        echo "${file}:${hit}"
    done <<< "$hits"
done < <(find src/stats -name '*.rs')

if [ "$status" -ne 0 ]; then
    cat >&2 <<'MSG'

src/stats must not perform I/O. Load the data in db/ and pass plain structs in.
See src/stats/mod.rs and docs/SPEC.md §8.
MSG
fi

exit "$status"
