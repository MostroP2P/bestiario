#!/usr/bin/env python3
"""The lines of a coverage report that no test executed.

Reads the JSON `cargo llvm-cov --json` writes and prints, for the paths
asked about, every executable line whose execution count is zero.

# Reading llvm's segments

A segment is not a line: it is the point where the execution count
*changes*, and that count holds from there until the next segment
(https://llvm.org/docs/doxygen/structllvm_1_1coverage_1_1CoverageSegment.html).
The count therefore has to be carried across the span rather than read off
the segment's own line. An unentered `if` body whose opening line also
carries a covered segment would otherwise look covered, and the lines
inside it would never be looked at at all — which is a gate that passes
without seeing.

A segment with no count, or one marking a gap, describes something that is
not executable there; neither can make a line uncovered.

# Why llvm's own list is checked against this one

Given `--llvm <file>`, the `Uncovered Lines:` section `cargo llvm-cov
--show-missing-lines` prints is compared with this walk, and a difference
in either direction fails. Two independent computations of the same thing
is the only guard against the failure this gate keeps producing: an
implementation that quietly sees nothing and reports success.

llvm's *per-file percentage* is deliberately not the gate. On this tree it
calls five files one or two lines short while llvm's own list of uncovered
lines — and this walk — name none, and a gate on a number that points at
no line can only ever block. The percentage is still printed by
`scripts/coverage.sh` for the workspace as a whole, where it is the figure
SPEC §12 sets at 95%.

Usage: uncovered.py <report.json> [--llvm <missing.txt>] <fragment> […]
Exits 1 when any line is uncovered, after listing it.
"""

import json
import sys


def line_counts(entry: dict) -> dict[int, int]:
    """The execution count of every executable line of `entry`.

    Each segment's count holds from its own line until the line the next
    segment starts on; where several spans cover one line, the largest
    count wins, which is how llvm itself folds regions into lines.
    """
    segments = sorted(entry.get("segments", []), key=lambda segment: (segment[0], segment[1]))
    counts: dict[int, int] = {}

    for index, (line, _column, count, has_count, _is_entry, is_gap) in enumerate(segments):
        if not has_count or is_gap:
            continue

        following = segments[index + 1][0] if index + 1 < len(segments) else line + 1
        for covered in range(line, max(following, line + 1)):
            counts[covered] = max(counts.get(covered, 0), count)

    return counts


def uncovered_lines(entry: dict) -> list[int]:
    """Line numbers in `entry` that no test executed."""
    return sorted(line for line, count in line_counts(entry).items() if count == 0)


def llvm_missing(path: str) -> dict[str, list[int]]:
    """The `Uncovered Lines:` section of a `--show-missing-lines` run.

    Every line of it reads `<absolute path>: 12, 34, 56`; anything before
    the heading is the per-file table, which this does not read.
    """
    missing: dict[str, list[int]] = {}
    with open(path, encoding="utf-8") as handle:
        listing = handle.read().split("Uncovered Lines:", 1)
        if len(listing) < 2:
            return missing

        for row in listing[1].splitlines():
            filename, separator, lines = row.partition(": ")
            if not separator or not filename.startswith("/"):
                continue
            missing[filename] = [int(line) for line in lines.replace(",", "").split()]

    return missing


def disagreement(filename: str, walked: list[int], llvm: dict[str, list[int]]) -> str | None:
    """How this walk differs from llvm's own list, if it does."""
    theirs = llvm.get(filename, [])
    if theirs == walked:
        return None

    return (
        f"llvm reports {theirs or 'no'} uncovered line(s) here and this walk "
        f"found {walked or 'none'}; the gate cannot say which is right"
    )


def short(filename: str) -> str:
    """The path as the repository names it."""
    return filename.split("/bestiario/")[-1]


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2

    report, rest = argv[1], argv[2:]
    llvm: dict[str, list[int]] = {}
    if rest[:1] == ["--llvm"]:
        llvm, rest = llvm_missing(rest[1]), rest[2:]
    fragments = rest
    if not fragments:
        print(__doc__, file=sys.stderr)
        return 2

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

        gaps: dict[str, list[int]] = {}
        confusions: dict[str, str] = {}
        for entry in measured:
            uncovered = uncovered_lines(entry)
            if uncovered:
                gaps[entry["filename"]] = uncovered
            if (confused := disagreement(entry["filename"], uncovered, llvm)) is not None:
                confusions[entry["filename"]] = confused

        if gaps:
            print(f"  {fragment} — {len(measured)} files measured, and these lines ran zero times:")
            for filename, lines in sorted(gaps.items()):
                print(f"    {short(filename)}: {', '.join(str(line) for line in lines)}")
            status = 1

        if confusions:
            print(f"  {fragment} — this walk and llvm's totals disagree:")
            for filename, confused in sorted(confusions.items()):
                print(f"    {short(filename)}: {confused}")
            status = 1

        if not gaps and not confusions:
            print(f"  {fragment} — {len(measured)} files, every line executed")

    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv))
