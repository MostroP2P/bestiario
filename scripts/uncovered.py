#!/usr/bin/env python3
"""The lines of a coverage report that no test executed.

Reads the JSON `cargo llvm-cov --json` writes and prints, for the paths
asked about, every line whose regions all have an execution count of zero.

That is the same signal `--show-missing-lines` and the annotated report
give, and deliberately not `summary.lines.percent`: llvm's per-file
summary can fall a line short of 100% on a file whose every line the
annotated report shows as executed, and a gate on a number nobody can
act on is a gate that only ever blocks.

Usage: uncovered.py <report.json> <path-fragment> [<path-fragment>…]
Exits 1 when any line is uncovered, after listing it.
"""

import json
import sys


def uncovered_lines(entry: dict) -> list[int]:
    """Line numbers in `entry` whose regions all ran zero times."""
    counts: dict[int, list[int]] = {}
    for line, _column, count, has_count, _entry, is_gap in entry.get("segments", []):
        if has_count and not is_gap:
            counts.setdefault(line, []).append(count)

    return sorted(line for line, seen in counts.items() if max(seen) == 0)


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2

    report, fragments = argv[1], argv[2:]
    with open(report, encoding="utf-8") as handle:
        files = json.load(handle)["data"][0]["files"]

    status = 0
    for fragment in fragments:
        measured = [entry for entry in files if fragment in entry["filename"]]

        # A fragment matching nothing is a gate that has stopped guarding:
        # a moved directory would otherwise read as "fully covered".
        if not measured:
            print(f"  {fragment} matched no measured file — the gate is not looking at it")
            status = 1
            continue

        gaps = {
            entry["filename"]: lines
            for entry in measured
            if (lines := uncovered_lines(entry))
        }
        if gaps:
            print(f"  {fragment} — {len(measured)} files measured, and these lines ran zero times:")
            for filename, lines in sorted(gaps.items()):
                short = filename.split("/bestiario/")[-1]
                print(f"    {short}: {', '.join(str(line) for line in lines)}")
            status = 1
        else:
            print(f"  {fragment} — {len(measured)} files, every line executed")

    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv))
