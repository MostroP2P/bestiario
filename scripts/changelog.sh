#!/usr/bin/env bash
# Release notes, derived from the conventional commits since the previous tag.
#
# The log is the only source. `feat:`, `fix:`, `docs:` … are already required
# of every commit (AGENTS.md), so the section a release ships can be read off
# the history instead of being written a second time by hand.
#
#   scripts/changelog.sh notes  <version> [<from-ref>]  # the body, to stdout
#   scripts/changelog.sh update <version> [<from-ref>]  # prepend it to CHANGELOG.md
#
# `update` is the hook cargo-release runs (release.toml) before it commits the
# version bump, so the tag it then creates already contains its own entry. It
# is idempotent: a CHANGELOG.md that already carries the version is left
# untouched, which is what makes the hook safe to run once per workspace crate.
# With DRY_RUN=true — as `cargo release` without `--execute` sets it — it
# prints what it would prepend and writes nothing.
#
# <from-ref> defaults to the newest `v*` tag reachable from HEAD other than the
# version being released, so the same command works before the tag exists and
# after CI has checked it out.
set -euo pipefail

usage() {
	echo "usage: ${0##*/} {notes|update} <version> [<from-ref>]" >&2
	exit 2
}

mode=${1:-}
version=${2:-}
from=${3:-}
[ -n "$mode" ] && [ -n "$version" ] || usage
case "$mode" in
notes | update) ;;
*) usage ;;
esac

root=$(git rev-parse --show-toplevel)
cd "$root"

if [ -z "$from" ]; then
	from=$(git tag --list 'v*' --sort=-v:refname --merged HEAD |
		grep -vFx "v$version" | head -n 1 || true)
fi

# No previous tag means the first release: everything up to HEAD is new.
if [ -n "$from" ]; then
	range="$from..HEAD"
else
	range="HEAD"
fi

# Sections in the order they are printed, and the commit types each collects.
# Anything else — including a subject that is not a conventional commit at all
# — falls through to "Other changes" rather than being dropped silently.
sections=(
	"breaking:Breaking changes"
	"feat:Features"
	"fix:Fixes"
	"perf:Performance"
	"refactor:Refactoring"
	"docs:Documentation"
	"test:Tests"
	"build:Build and CI"
	"ci:Build and CI"
	"chore:Chores"
	"other:Other changes"
)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

bucket_of() {
	case "$1" in
	feat) echo feat ;;
	fix) echo fix ;;
	perf) echo perf ;;
	refactor) echo refactor ;;
	docs) echo docs ;;
	test) echo test ;;
	build) echo build ;;
	ci) echo ci ;;
	chore) echo chore ;;
	*) echo other ;;
	esac
}

while IFS=$'\t' read -r hash short subject; do
	[ -n "$hash" ] || continue
	type=""
	scope=""
	bang=""
	text=$subject
	if [[ $subject =~ ^([a-zA-Z]+)(\(([^\)]+)\))?(!)?:[[:space:]]+(.+)$ ]]; then
		type=$(printf '%s' "${BASH_REMATCH[1]}" | tr '[:upper:]' '[:lower:]')
		scope=${BASH_REMATCH[3]}
		bang=${BASH_REMATCH[4]}
		text=${BASH_REMATCH[5]}
	fi

	# The release commit cargo-release itself writes says nothing a reader of
	# the release wants; every other chore does.
	[ "$type" = "chore" ] && [ "$scope" = "release" ] && continue

	bucket=$(bucket_of "$type")
	if [ -n "$bang" ] || git log -1 --format='%b' "$hash" | grep -q '^BREAKING[ -]CHANGE'; then
		bucket=breaking
	fi

	if [ -n "$scope" ]; then
		printf -- '- **%s**: %s (%s)\n' "$scope" "$text" "$short" >>"$work/$bucket"
	else
		printf -- '- %s (%s)\n' "$text" "$short" >>"$work/$bucket"
	fi
done < <(git log --no-merges --reverse --format='%H%x09%h%x09%s' "$range")

body=$work/body
: >"$body"
seen_titles=""
for section in "${sections[@]}"; do
	bucket=${section%%:*}
	title=${section#*:}
	[ -s "$work/$bucket" ] || continue
	# build and ci share a title; open the heading only the first time.
	if [[ $seen_titles != *"|$title|"* ]]; then
		[ -s "$body" ] && echo >>"$body"
		printf '### %s\n\n' "$title" >>"$body"
		seen_titles="$seen_titles|$title|"
	fi
	cat "$work/$bucket" >>"$body"
done

if [ ! -s "$body" ]; then
	printf 'No commits since %s.\n' "${from:-the start of history}" >>"$body"
fi

if [ "$mode" = notes ]; then
	cat "$body"
	exit 0
fi

changelog=$root/CHANGELOG.md
if [ -f "$changelog" ] && grep -qF "## [$version]" "$changelog"; then
	echo "changelog: CHANGELOG.md already has $version, leaving it alone" >&2
	exit 0
fi

entry=$work/entry
{
	printf '## [%s] - %s\n\n' "$version" "$(date -u +%Y-%m-%d)"
	cat "$body"
} >"$entry"

if [ "${DRY_RUN:-false}" = "true" ]; then
	echo "changelog: dry run, CHANGELOG.md would gain:" >&2
	cat "$entry"
	exit 0
fi

header='# Changelog

Every release of bestiario, newest first. The entries are generated from the
conventional commits of `AGENTS.md` by `scripts/changelog.sh`; edit the log,
not this file.
'

merged=$work/CHANGELOG.md
if [ -f "$changelog" ]; then
	# Keep the existing preamble, and insert above the newest release heading.
	awk -v entry="$entry" '
		BEGIN { inserted = 0 }
		/^## \[/ && !inserted { while ((getline line < entry) > 0) print line; print ""; inserted = 1 }
		{ print }
		END { if (!inserted) { print ""; while ((getline line < entry) > 0) print line } }
	' "$changelog" >"$merged"
else
	{
		printf '%s\n' "$header"
		cat "$entry"
	} >"$merged"
fi

cp "$merged" "$changelog"
echo "changelog: CHANGELOG.md now carries $version" >&2
