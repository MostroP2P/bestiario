#!/usr/bin/env bash
#
# Exercises the container image, because building it proves almost nothing.
#
# Both container bugs this deployment has had passed every check that existed
# at the time. A stale-mtime fingerprint once shipped a stub binary that
# started, printed nothing and exited 0: `docker build` succeeded and all
# 1013 Rust tests passed. Only running the thing found it. Every assertion
# below is one of those failures turned into a test.
#
# Deliberately offline: nothing here dials a relay, so the suite cannot fail
# because someone else's infrastructure is having a bad morning.
#
# Usage: scripts/smoke-image.sh [IMAGE]

set -euo pipefail

IMAGE="${1:-bestiario:ci}"

readonly INSTANCE=82fa8cb978b43c79b2156585bac2c011176a21d2aead6d9f7c575c005be88390

failures=0

# Runs the image and checks its exit code and combined output.
#
# Arguments: name, expected status, expected text, then the docker options, a
# literal `--`, and the container's own arguments. The separator is not
# decoration: docker options go before the image name and the container's
# arguments after it, and collapsing the two is how the first draft of this
# script managed to ask docker for a `--version` flag it does not have.
#
# Output is captured rather than streamed so the assertion can read it; on
# failure the whole thing is printed, since a smoke test that says only
# "failed" sends you back to running it by hand.
check() {
    local name="$1" want_status="$2" want_text="$3"
    shift 3

    local opts=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do
        opts+=("$1")
        shift
    done
    shift  # the separator itself

    local output status=0
    output="$(docker run --rm ${opts[@]+"${opts[@]}"} "$IMAGE" "$@" 2>&1)" || status=$?

    if [[ "$status" != "$want_status" ]]; then
        printf 'FAIL %s: exited %s, expected %s\n%s\n\n' \
            "$name" "$status" "$want_status" "$output" >&2
        failures=$((failures + 1))
        return
    fi

    if [[ "$output" != *"$want_text"* ]]; then
        printf 'FAIL %s: output does not contain %q\n%s\n\n' \
            "$name" "$want_text" "$output" >&2
        failures=$((failures + 1))
        return
    fi

    printf 'ok   %s\n' "$name"
}

# The stub-binary regression. A binary that answers `--version` is a binary
# that was actually linked; the stub answered nothing and exited 0, so this
# assertion is the one that would have caught it.
check "the binary is the real one, not a build stub" 0 "bestiario" -- --version

# litestream has to be present and executable, and of the pinned version. A
# cross-architecture mismatch produces an exec-format error here rather than
# at three in the morning on the first restart.
check "litestream is installed and runs" 0 "0.5" --entrypoint litestream -- version

# No configuration at all must fail loudly. The whole reason the image ships
# no settings.toml is that validation still refuses an empty configuration;
# if this ever exits 0, an operator can deploy a worker that indexes nothing.
check "an unconfigured image refuses to start" 1 "[nostr].relays is empty" -- summary

# Configuration from the environment alone, including a comma-separated list.
# Reaching "no events yet" means the settings parsed, validated, and the
# database was created and migrated — everything short of the network.
check "the environment alone configures the daemon" 1 "database holds no events yet" \
    -e "BESTIARIO__NOSTR__RELAYS=wss://relay.example,wss://relay.invalid" \
    -e "BESTIARIO__INDEXER__INSTANCES=$INSTANCE" \
    -- summary

# The wrapper is the entrypoint, so an image without a bucket has to fall
# through to the plain daemon rather than refusing to run or, worse, claiming
# to replicate.
check "without a bucket the wrapper runs the daemon unreplicated" 1 \
    "LITESTREAM_BUCKET is unset" \
    -e "BESTIARIO__NOSTR__RELAYS=wss://relay.example" \
    -e "BESTIARIO__INDEXER__INSTANCES=$INSTANCE" \
    -- summary

if [[ "$failures" -ne 0 ]]; then
    printf '\n%s smoke check(s) failed\n' "$failures" >&2
    exit 1
fi

printf '\nimage smoke checks passed\n'
