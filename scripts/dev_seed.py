#!/usr/bin/env python3
"""Populate an isolated else development runtime through its public CLI."""

import json
import subprocess
import sys
from typing import Any

PENDING = "[dev] Review the launch checklist"
COMPLETED = "[dev] Publish the weekly product briefing"
WAITING = "[dev] Incorporate partner feedback"


def run(binary: str, runtime: str, *arguments: str) -> Any:
    process = subprocess.run(
        [binary, "--dir", runtime, *arguments, "--json"],
        text=True,
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        raise RuntimeError(
            f"else-bot {' '.join(arguments)} failed ({process.returncode}):\n"
            f"{process.stderr.strip()}"
        )
    try:
        return json.loads(process.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"else-bot {' '.join(arguments)} returned invalid JSON: {process.stdout!r}"
        ) from error


def create(binary: str, runtime: str, outcome: str) -> dict[str, Any]:
    snapshot = run(
        binary,
        runtime,
        "intent",
        outcome,
        "--project",
        "dev-demo",
        "--harness",
        "scripted",
    )
    if not snapshot["decisions"]:
        raise RuntimeError(f"seed intent did not produce a decision: {outcome}")
    return snapshot


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: dev_seed.py <else-bot-binary> <runtime-dir>")
    binary, runtime = sys.argv[1:]
    existing = {
        item["spec"]["outcome"]: item for item in run(binary, runtime, "list")
    }

    if PENDING not in existing:
        snapshot = create(binary, runtime, PENDING)
        existing[PENDING] = snapshot
        print(f"created decision: {snapshot['spec']['id']}")
    else:
        print(f"kept decision:    {existing[PENDING]['spec']['id']}")

    if COMPLETED not in existing:
        snapshot = create(binary, runtime, COMPLETED)
        intent_id = snapshot["spec"]["id"]
        request_id = snapshot["decisions"][0]["id"]
        snapshot = run(binary, runtime, "decide", intent_id, request_id, "approve")
        if snapshot["state"] != "completed":
            raise RuntimeError(f"seed result did not complete: {snapshot['state']}")
        existing[COMPLETED] = snapshot
        print(f"created result:   {intent_id}")
    else:
        print(f"kept result:      {existing[COMPLETED]['spec']['id']}")

    if WAITING not in existing:
        snapshot = create(binary, runtime, WAITING)
        intent_id = snapshot["spec"]["id"]
        snapshot = run(
            binary,
            runtime,
            "wait-external",
            intent_id,
            "--reason",
            "Partner feedback is expected tomorrow",
        )
        existing[WAITING] = snapshot
        print(f"created waiting:  {intent_id}")
    else:
        print(f"kept waiting:     {existing[WAITING]['spec']['id']}")

    print("\nDevelopment data is ready. Run scripts/dev.sh tui to inspect it.")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, IndexError, RuntimeError) as error:
        print(f"dev seed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
