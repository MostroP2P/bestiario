#!/usr/bin/env bash
#
# The aggregation layer must perform no I/O, so that an HTTP API can reuse it
# unchanged (docs/SPEC.md §8). That rule is enforced primarily by cargo:
# bestiario-stats is its own crate, so a crate it does not depend on is not in
# scope and code that reaches for one does not compile.
#
# This script guards the remaining hole — someone adding an I/O crate to the
# manifest. It checks every dependency section of the stats crate against an
# allowlist of computation-only crates.
#
# Adding to ALLOWED is allowed. It just has to be a deliberate act that shows
# up in review, with a reason why the crate performs no I/O.

set -euo pipefail

MANIFEST="crates/stats/Cargo.toml"
ALLOWED=(chrono serde serde_json thiserror uuid)

# Every [dependencies], [dev-dependencies], [build-dependencies] and
# [target.*.dependencies] table, reduced to the crate names they declare.
declared=$(
    awk '/^\[[^]]*dependencies\]/ { in_deps = 1; next }
         /^\[/                    { in_deps = 0 }
         in_deps' "$MANIFEST" |
        grep -vE '^[[:space:]]*(#|$)' |
        sed -E 's/^[[:space:]]*([A-Za-z0-9_-]+).*/\1/'
)

status=0
while IFS= read -r dep; do
    [ -n "$dep" ] || continue
    if ! printf '%s\n' "${ALLOWED[@]}" | grep -qx -- "$dep"; then
        echo "${MANIFEST}: '${dep}' is not on the no-I/O allowlist"
        status=1
    fi
done <<< "$declared"

if [ "$status" -ne 0 ]; then
    cat >&2 <<MSG

crates/stats must perform no I/O: load the data in db/ and pass plain structs
in. If the crate above genuinely performs no I/O, add it to ALLOWED in
$0 and say why in the pull request.
MSG
fi

exit "$status"
