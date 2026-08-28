#!/usr/bin/env python3
"""Refuses a shell operator in the App Platform spec's `run_command`.

This exists because of a failure that cost two deployments. `run_command:
bestiario backfill && bestiario sync` passes `doctl apps spec validate` — the
schema is fine, a string is a string — and then fails at run time with
DeployContainerExitNonZero, because App Platform does not guarantee that
run_command reaches a shell. App Platform rolls back, so the app stays up
running the previous command and nothing looks broken.

The spec is parsed rather than grepped. A first version of this check read
only the line the key appears on, which a block scalar walks straight past:

    run_command: >-
      bestiario backfill && bestiario sync

Parsing also means YAML comments are gone before anything is inspected, so a
comment that mentions an operator is not mistaken for a command.

Sequencing belongs in deploy/replicated.sh, where a shell is guaranteed.
See docs/DEPLOY.md.
"""

import sys
from pathlib import Path

import yaml

SPEC = Path(__file__).resolve().parent.parent / ".do" / "app.yaml"

# `|` is included for the pipe, which is as unavailable as the rest; `\n`
# because a multi-line command is a script, and a script belongs in a file.
OPERATORS = ("&&", "||", ";", "|", "&", "\n")


def run_commands(node, path="spec"):
    """Yields every (location, value) of a `run_command` key in the document.

    Walks the whole structure rather than the component lists by name, so a
    key added under a component type this script has never heard of is still
    checked.
    """
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "run_command" and isinstance(value, str):
                yield f"{path}.{key}", value
            else:
                yield from run_commands(value, f"{path}.{key}")
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from run_commands(value, f"{path}[{index}]")


def main():
    if not SPEC.exists():
        sys.exit(f"{SPEC}: not found")

    spec = yaml.safe_load(SPEC.read_text(encoding="utf-8"))

    offenders = [
        (where, command, operator)
        for where, command in run_commands(spec)
        for operator in OPERATORS
        if operator in command
    ]

    if offenders:
        for where, command, operator in offenders:
            print(
                f"{SPEC.name}: {where} contains {operator!r}: {command!r}",
                file=sys.stderr,
            )
        sys.exit(
            "\nrun_command must be a single command, with no shell operators.\n"
            "App Platform does not guarantee it reaches a shell, so the operator\n"
            "is never interpreted and the container exits before the daemon\n"
            "starts. Sequence the commands in deploy/replicated.sh instead."
        )

    checked = sum(1 for _ in run_commands(spec))
    print(f"{SPEC.name}: {checked} run_command(s), none carrying a shell operator")


if __name__ == "__main__":
    main()
