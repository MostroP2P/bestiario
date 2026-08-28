#!/usr/bin/env bash
# Tests for scripts/changelog.sh, over a throwaway repository whose history is
# written here. What is checked is what a release depends on: the commits since
# the previous tag and no others, grouped by conventional-commit type, with the
# release commit left out and a breaking change lifted to the top; and that the
# CHANGELOG.md update is idempotent, because cargo-release runs the hook once
# per crate of the workspace.
set -euo pipefail

# The fixture repository must not inherit the maintainer's git configuration:
# a global `tag.gpgSign` or commit template would decide the outcome of a test
# about this repository's changelog.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

script=$(cd "$(dirname "$0")" && pwd)/changelog.sh
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
contains() {
	local what=$1 needle=$2 haystack=$3
	case "$haystack" in
	*"$needle"*) echo "ok   $what" ;;
	*)
		echo "FAIL $what" >&2
		echo "  missing: $needle" >&2
		echo "  in:      $haystack" >&2
		failures=$((failures + 1))
		;;
	esac
}

commit() {
	git -C "$tmp" commit --allow-empty -q -m "$1"
}

git -C "$tmp" init -q
git -C "$tmp" config user.email bestiario@example.invalid
git -C "$tmp" config user.name bestiario

commit "feat(ingest): before the previous release"
git -C "$tmp" tag -a v0.1.0 -m v0.1.0
commit "feat(publish): the document model"
commit "fix: a subject with no scope"
commit "refactor(publish)!: the key is an environment variable"
commit "docs(readme): how to release"
commit "chore(release): 0.2.0"
commit "not a conventional commit"

cd "$tmp"

notes=$("$script" notes 0.2.0)

check "no commit before the previous tag" "0" \
	"$(grep -c 'before the previous release' <<<"$notes" || true)"
contains "a scope is bolded" "- **publish**: the document model" "$notes"
contains "a scopeless subject stands alone" "- a subject with no scope" "$notes"
contains "features get their heading" "### Features" "$notes"
contains "a breaking change is lifted" "### Breaking changes" "$notes"
check "the breaking change leads" "### Breaking changes" "$(head -n 1 <<<"$notes")"
check "the release commit is dropped" "0" \
	"$(grep -c 'chore(release)' <<<"$notes" || true)"
contains "a non-conventional subject survives" "### Other changes" "$notes"
contains "and keeps its text" "- not a conventional commit" "$notes"

"$script" update 0.2.0 >/dev/null 2>&1
first=$(cat CHANGELOG.md)
contains "update writes the version heading" "## [0.2.0] - " "$first"
contains "update carries the body" "- **publish**: the document model" "$first"

"$script" update 0.2.0 >/dev/null 2>&1
check "update is idempotent" "$first" "$(cat CHANGELOG.md)"

git -C "$tmp" add -A
git -C "$tmp" commit -q -m "chore(release): 0.2.0"
git -C "$tmp" tag -a v0.2.0 -m v0.2.0
commit "fix(cli): after the release"
"$script" update 0.3.0 >/dev/null 2>&1
check "a later entry goes on top" "## [0.3.0] - $(date -u +%Y-%m-%d)" \
	"$(grep -m 1 '^## \[' CHANGELOG.md)"
check "the older entry stays" "1" \
	"$(grep -c '^## \[0.2.0\]' CHANGELOG.md)"
contains "and only the new commits are in the new entry" "- **cli**: after the release" \
	"$("$script" notes 0.3.0)"
check "the new entry stops at the previous tag" "0" \
	"$("$script" notes 0.3.0 | grep -c 'the document model' || true)"

if [ "$failures" -ne 0 ]; then
	echo "$failures check(s) failed" >&2
	exit 1
fi
echo "changelog: all checks passed"
