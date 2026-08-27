#!/usr/bin/env python3
"""Checks the dependency graph and the phase map of docs/ROADMAP.md.

Every check here exists because a human reader missed the thing it checks:

* The phase map claimed ranges that disagreed with the detailed tables.
* Rows needed earlier rows without saying so, in a document whose own
  conventions state that adjacency means nothing.

Both are cheap to verify and expensive to notice by eye.
"""

import re
import sys
from pathlib import Path

ROADMAP = Path(__file__).resolve().parent.parent / "docs" / "ROADMAP.md"

# The last required row: the coverage pass that measures the finished project.
# Rows after it are the optional phases that follow — the HTTP API and the
# Nostr publication, neither of which the coverage figure is measuring.
TERMINAL = 45

# `| 07 | title | S | 01, 03 | scope |` — a row of one of the detailed tables.
DETAIL_ROW = re.compile(r"^\| (\d\d) \| (?:[^|]*\| ){2}([^|]*)\|")
# `| 1 | Ingestion | 07–22 | exit criterion |` — a row of the phase map.
PHASE_ROW = re.compile(r"^\| (\d) \| [^|]* \| (\d\d(?:–\d\d)?) \|")


def parse(text):
    """Returns the detailed rows, any duplicated numbers, and the phase map."""
    rows, duplicates, declared = {}, [], set()

    for line in text.splitlines():
        detail = DETAIL_ROW.match(line)
        if detail:
            number = int(detail.group(1))
            if number in rows:
                duplicates.append(number)
            rows[number] = {int(d) for d in re.findall(r"\b\d\d\b", detail.group(2))}
            continue

        phase = PHASE_ROW.match(line)
        if phase:
            bounds = [int(b) for b in phase.group(2).split("–")]
            first, last = bounds[0], bounds[-1]
            declared.update(range(first, last + 1))

    return rows, duplicates, declared


def reachable_from(rows, start):
    seen, stack = set(), [start]
    while stack:
        for dependency in rows.get(stack.pop(), ()):
            if dependency not in seen:
                seen.add(dependency)
                stack.append(dependency)
    return seen


def check(text):
    rows, duplicates, declared = parse(text)
    problems = []

    if duplicates:
        problems.append("rows defined more than once: " + numbers(duplicates))

    # The phase map is the declared contents of the plan, so the detailed
    # tables are checked against it rather than against their own length —
    # otherwise deleting a whole phase would pass unnoticed.
    if not declared:
        problems.append("no phase map found, so the row numbering cannot be checked")
    else:
        missing = declared - set(rows)
        extra = set(rows) - declared
        if missing:
            problems.append(f"the phase map declares rows with no detailed entry: {numbers(missing)}")
        if extra:
            problems.append(f"detailed rows that no phase declares: {numbers(extra)}")

    for number, dependencies in sorted(rows.items()):
        for dependency in sorted(dependencies):
            if dependency not in rows:
                problems.append(f"PR {number} depends on PR {dependency}, which does not exist")
            elif dependency >= number:
                problems.append(f"PR {number} depends on PR {dependency}, which does not precede it")

    if TERMINAL not in rows:
        problems.append(f"PR {TERMINAL} is missing; it is the row every earlier row must lead to")
    else:
        # Anything before the terminal row that it cannot reach could
        # legitimately merge afterwards, which would make the coverage figure a
        # measurement of an incomplete tree. Rows after it are the optional
        # phase, and are meant to come later.
        earlier = {n for n in rows if n < TERMINAL}
        unreachable = earlier - reachable_from(rows, TERMINAL)
        if unreachable:
            problems.append(
                f"PR {TERMINAL} does not transitively depend on: {numbers(unreachable)}"
                " — either add the dependency, or move the row after it"
            )

    return rows, problems


def numbers(values):
    return ", ".join(f"{n:02d}" for n in sorted(values))


def main():
    rows, problems = check(ROADMAP.read_text())

    if problems:
        print("docs/ROADMAP.md:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(
        f"docs/ROADMAP.md: {len(rows)} rows matching the phase map, graph acyclic,"
        f" every row before PR {TERMINAL} reachable from it"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
