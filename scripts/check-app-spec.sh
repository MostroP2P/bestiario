#!/usr/bin/env bash
#
# Refuses a shell operator in the App Platform spec's run_command.
#
# This encodes a failure that cost two deployments. `run_command: bestiario
# backfill && bestiario sync` passed `doctl apps spec validate` — the schema is
# fine, the string is a string — and then failed at run time with
# DeployContainerExitNonZero, because App Platform does not guarantee that
# run_command reaches a shell. App Platform rolled back, so the app stayed up
# and quietly went on running the previous command; nothing looked broken.
#
# The sequencing belongs in deploy/replicated.sh, where a shell is guaranteed.
# See docs/DEPLOY.md.

set -euo pipefail

readonly SPEC=".do/app.yaml"

if [[ ! -f "$SPEC" ]]; then
    printf '%s: not found\n' "$SPEC" >&2
    exit 1
fi

# Commented lines are excluded by the leading-whitespace anchor: the spec
# documents alternatives it does not apply, and a commented-out example is not
# a command anyone will run.
offenders="$(grep -nE '^[[:space:]]*run_command:' "$SPEC" \
    | grep -E '(\&\&|\|\||;|\|)' || true)"

if [[ -n "$offenders" ]]; then
    printf '%s: run_command must be a single command, with no shell operators.\n' "$SPEC" >&2
    printf 'App Platform does not guarantee it reaches a shell, so the operator\n' >&2
    printf 'is never interpreted and the container exits before the daemon starts.\n' >&2
    printf 'Sequence the commands in deploy/replicated.sh instead.\n\n' >&2
    printf '%s\n' "$offenders" >&2
    exit 1
fi

printf '%s: run_command carries no shell operators\n' "$SPEC"
