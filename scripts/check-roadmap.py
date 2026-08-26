#!/usr/bin/env python3
"""Checks the dependency graph of docs/ROADMAP.md.

Two review findings on the roadmap were both the same mistake: a row that
needed an earlier one without saying so, relying on adjacency in a document
that explicitly states adjacency means nothing. A reader will make that
mistake again; a script will not.
"""

import re
import sys
from pathlib import Path

ROADMAP = Path(__file__).resolve().parent.parent / "docs" / "ROADMAP.md"
# The final row: the coverage pass that measures the finished project.
TERMINAL = 45

ROW = re.compile(r"^\| (\d\d) \| (?:[^|]*\| ){2}([^|]*)\|")


def parse_rows(text):
    rows = {}
    for line in text.splitlines():
        match = ROW.match(line)
        if match:
            number = int(match.group(1))
            rows[number] = {int(d) for d in re.findall(r"\b\d\d\b", match.group(2))}
    return rows


def reachable_from(rows, start):
    seen, stack = set(), [start]
    while stack:
        current = stack.pop()
        for dependency in rows.get(current, ()):
            if dependency not in seen:
                seen.add(dependency)
                stack.append(dependency)
    return seen


def main():
    rows = parse_rows(ROADMAP.read_text())
    problems = []

    expected = list(range(1, len(rows) + 1))
    if sorted(rows) != expected:
        problems.append(f"numbering is not contiguous 1..{len(rows)}: {sorted(rows)}")

    for number, dependencies in sorted(rows.items()):
        for dependency in sorted(dependencies):
            if dependency not in rows:
                problems.append(f"PR {number} depends on PR {dependency}, which does not exist")
            elif dependency >= number:
                problems.append(f"PR {number} depends on PR {dependency}, which does not precede it")

    # Anything below the terminal row that it cannot reach could legitimately
    # merge after it, which would make the coverage figure a measurement of an
    # incomplete tree. Rows above it are the optional phase that follows and
    # are meant to come later.
    if TERMINAL in rows:
        earlier = {n for n in rows if n < TERMINAL}
        unreachable = earlier - reachable_from(rows, TERMINAL)
        if unreachable:
            problems.append(
                f"PR {TERMINAL} does not transitively depend on: "
                + ", ".join(str(n) for n in sorted(unreachable))
                + " — either add the dependency, or move the row after it"
            )

    if problems:
        print("docs/ROADMAP.md:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"docs/ROADMAP.md: {len(rows)} rows, graph is acyclic, contiguous and fully reachable from PR {TERMINAL}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
