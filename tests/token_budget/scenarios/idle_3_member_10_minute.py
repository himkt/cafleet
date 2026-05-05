"""Per-tick baseline measurement for design doc 0000049 (token reduction).

Spawns an isolated 3-member CAFleet session, lets the panes settle, then
captures + byte-counts the outputs that a Director's /loop tick currently
runs against its team:

    1. ``cafleet member list --agent-id <director>``
    2. ``cafleet message poll --agent-id <director>``
    3. ``cafleet member capture --agent-id <director> --member-id <m> --lines 200``
       (per member)

The total byte count is the per-tick cost the design doc's surface stack
must beat. Run this script BEFORE landing any surface; the result anchors
the regression budget in tests/token_budget/measurement_results.md.

Usage::

    uv run python tests/token_budget/scenarios/idle_3_member_10_minute.py

Must run inside a tmux session — ``cafleet member create`` requires it.
The script splits the calling pane's window with three temporary
``claude`` panes and tears them down on exit.
"""

from __future__ import annotations

import datetime
import json
import os
import subprocess
import sys
import time
from pathlib import Path

NUM_MEMBERS = 3
SETTLE_SECONDS = 30
CAPTURE_LINES = 200
IDLE_PROMPT = "You are an idle baseline member. Do nothing."
SESSION_LABEL = "baseline-0049"
RESULTS_KEY = "baseline_pre_design_0049_per_tick_bytes"

REPO_ROOT = Path(__file__).resolve().parents[3]
RESULTS_PATH = REPO_ROOT / "tests" / "token_budget" / "measurement_results.md"


def _run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=check, capture_output=True, text=True)


def _create_session() -> tuple[str, str]:
    proc = _run(["cafleet", "--json", "session", "create", "--label", SESSION_LABEL])
    payload = json.loads(proc.stdout)
    return payload["session_id"], payload["director"]["agent_id"]


def _spawn_member(session_id: str, director_id: str, name: str) -> str:
    proc = _run(
        [
            "cafleet",
            "--session-id",
            session_id,
            "--json",
            "member",
            "create",
            "--agent-id",
            director_id,
            "--name",
            name,
            "--description",
            "Idle baseline member for token-budget capture",
            "--",
            IDLE_PROMPT,
        ]
    )
    return json.loads(proc.stdout)["agent_id"]


def _capture_director_view(
    session_id: str, director_id: str, member_ids: list[str]
) -> list[tuple[str, str]]:
    captured: list[tuple[str, str]] = []

    proc = _run(
        ["cafleet", "--session-id", session_id, "member", "list", "--agent-id", director_id]
    )
    captured.append(("member list", proc.stdout))

    proc = _run(
        ["cafleet", "--session-id", session_id, "message", "poll", "--agent-id", director_id]
    )
    captured.append(("message poll", proc.stdout))

    for member_id in member_ids:
        proc = _run(
            [
                "cafleet",
                "--session-id",
                session_id,
                "member",
                "capture",
                "--agent-id",
                director_id,
                "--member-id",
                member_id,
                "--lines",
                str(CAPTURE_LINES),
            ]
        )
        captured.append((f"member capture {member_id[:8]}", proc.stdout))

    return captured


def _teardown(session_id: str, director_id: str, member_ids: list[str]) -> None:
    for member_id in member_ids:
        _run(
            [
                "cafleet",
                "--session-id",
                session_id,
                "member",
                "delete",
                "--agent-id",
                director_id,
                "--member-id",
                member_id,
                "--force",
            ],
            check=False,
        )
    _run(["cafleet", "session", "delete", session_id], check=False)


def _write_results(
    session_id: str,
    director_id: str,
    member_ids: list[str],
    breakdown: list[tuple[str, int]],
    total_bytes: int,
) -> None:
    timestamp = datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds")
    breakdown_lines = "\n".join(f"- {label}: {n} bytes" for label, n in breakdown)
    content = (
        "# Token-budget measurement results — design doc 0000049\n"
        "\n"
        "Per-tick byte counts of the Director monitoring commands run against an\n"
        "isolated 3-member idle CAFleet session. Updated by\n"
        "`tests/token_budget/scenarios/idle_3_member_10_minute.py`.\n"
        "\n"
        "## Pre-design-0049 baseline\n"
        "\n"
        f"- captured_at: {timestamp}\n"
        "- scenario: idle_3_member_10_minute\n"
        f"- session_id: {session_id}\n"
        f"- director_agent_id: {director_id}\n"
        f"- member_count: {len(member_ids)}\n"
        f"- capture_lines: {CAPTURE_LINES}\n"
        f"- settle_seconds: {SETTLE_SECONDS}\n"
        "\n"
        "Per-command byte counts:\n"
        "\n"
        f"{breakdown_lines}\n"
        "\n"
        f"{RESULTS_KEY}: {total_bytes}\n"
    )
    RESULTS_PATH.parent.mkdir(parents=True, exist_ok=True)
    RESULTS_PATH.write_text(content)


def main() -> int:
    if "TMUX" not in os.environ:
        print(
            "error: must run inside a tmux session "
            "(cafleet member create needs it).",
            file=sys.stderr,
        )
        return 1

    print(f"Creating baseline session (label={SESSION_LABEL!r})...")
    session_id, director_id = _create_session()
    print(f"  session_id        = {session_id}")
    print(f"  director_agent_id = {director_id}")

    member_ids: list[str] = []
    try:
        for i in range(1, NUM_MEMBERS + 1):
            name = f"idle-{i}"
            print(f"Spawning member {name}...")
            member_id = _spawn_member(session_id, director_id, name)
            member_ids.append(member_id)
            print(f"  agent_id = {member_id}")

        print(f"Waiting {SETTLE_SECONDS}s for panes to settle...")
        time.sleep(SETTLE_SECONDS)

        print("Capturing Director view (member list / message poll / per-member capture)...")
        captured = _capture_director_view(session_id, director_id, member_ids)

        breakdown = [(label, len(text.encode("utf-8"))) for label, text in captured]
        total_bytes = sum(n for _, n in breakdown)

        print("Per-command byte counts:")
        for label, n in breakdown:
            print(f"  {label}: {n} bytes")
        print(f"Total per-tick bytes: {total_bytes}")

        _write_results(session_id, director_id, member_ids, breakdown, total_bytes)
        print(f"Wrote {RESULTS_PATH}")
    finally:
        print("Tearing down...")
        _teardown(session_id, director_id, member_ids)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
