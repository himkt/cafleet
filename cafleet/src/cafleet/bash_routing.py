"""Bash-routing payload helpers + allow×deny matcher (design doc 0000034).

Members spawned with ``--no-bash`` route shell commands through their Director
via a JSON ``bash_request`` envelope. The Director matches the request against
its resolved ``permissions.allow`` / ``permissions.deny``, runs the command via
``cafleet bash-exec``, and replies with a ``bash_result``. This module exposes
three pure helpers used by that flow:

- ``parse_bash_request`` — JSON-shape discriminator. Returns ``None`` for
  non-``bash_request`` shapes; does NOT validate field semantics (empty cmd,
  oversized timeout) — those are the helper's responsibility.
- ``format_bash_result`` — pure formatter. Caller passes already-truncated
  streams; the formatter does NOT truncate.
- ``match_allow`` — applies the §4 allow×deny truth table. Returns
  ``"auto-run"`` only when allow matches AND deny does not. Non-``Bash(...)``
  patterns are ignored. No ``"auto-deny"`` outcome.

The ``BashRequest`` / ``BashResult`` types are ``TypedDict``s so the parsed
payload is already a mapping at runtime — tests subscript directly without
``dataclasses.asdict``.
"""

import fnmatch
import json
from typing import Literal, TypedDict


class BashRequest(TypedDict, total=False):
    type: Literal["bash_request"]
    cmd: str
    cwd: str | None
    stdin: str | None
    timeout: int
    reason: str


class BashResult(TypedDict, total=False):
    type: Literal["bash_result"]
    in_reply_to: str
    status: Literal["ran", "denied", "timeout"]
    exit_code: int
    stdout: str
    stderr: str
    duration_ms: int
    note: str


def parse_bash_request(text: str) -> BashRequest | None:
    """Parse a polled ``text`` body into a ``BashRequest``.

    Returns ``None`` for non-``bash_request`` shapes (parse fail / not a JSON
    object / missing ``type`` / ``type != 'bash_request'``). Does NOT validate
    field semantics — empty ``cmd`` and oversized ``timeout`` are returned
    verbatim and rejected later by ``cafleet bash-exec``.
    """
    try:
        parsed = json.loads(text)
    except (json.JSONDecodeError, ValueError):
        return None
    if not isinstance(parsed, dict):
        return None
    if parsed.get("type") != "bash_request":
        return None
    return parsed  # type: ignore[return-value]


def format_bash_result(
    *,
    in_reply_to: str,
    status: Literal["ran", "denied", "timeout"],
    exit_code: int,
    stdout: str = "",
    stderr: str = "",
    duration_ms: int = 0,
    note: str | None = None,
) -> str:
    """Return a JSON-encoded ``bash_result`` payload.

    Pure formatter — does NOT truncate; the caller passes already-truncated
    streams. Per the canonical-status rule, ``status`` is the sole source of
    truth; ``exit_code`` is opaque on ``denied`` / ``timeout`` outcomes but
    the formatter still serializes whatever the caller provides.

    The ``note`` key is OMITTED from the output when ``note=None`` (key
    absent, not ``null``) — design §3 ``bash_result`` payload: "Omitted
    otherwise."
    """
    payload: dict[str, object] = {
        "type": "bash_result",
        "in_reply_to": in_reply_to,
        "status": status,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "duration_ms": duration_ms,
    }
    if note is not None:
        payload["note"] = note
    return json.dumps(payload)


def _extract_bash_glob(pattern: str) -> str | None:
    """Return the inner glob of a ``Bash(...)`` pattern, or ``None`` for non-Bash.

    Only patterns of the shape ``Bash(<glob>)`` are considered — other tool
    patterns (``Edit``, ``Skill(...)``, etc.) are ignored. Malformed shapes
    (missing closing paren, etc.) are skipped silently.
    """
    if not pattern.startswith("Bash(") or not pattern.endswith(")"):
        return None
    return pattern[len("Bash(") : -1]


def _any_bash_pattern_matches(cmd: str, patterns: list[str]) -> bool:
    for pattern in patterns:
        glob = _extract_bash_glob(pattern)
        if glob is None:
            continue
        if fnmatch.fnmatchcase(cmd, glob):
            return True
    return False


def match_allow(
    cmd: str,
    allow_patterns: list[str],
    deny_patterns: list[str],
) -> Literal["auto-run", "ask"]:
    """Apply the §4 allow×deny truth table.

    Returns ``"auto-run"`` when an allow pattern matches AND no deny pattern
    matches. Returns ``"ask"`` for every other combination. Only patterns of
    the shape ``Bash(<glob>)`` are considered. There is no ``"auto-deny"``
    outcome — ``permissions.deny`` only downgrades a would-be ``auto-run``
    to ``ask``; the operator can still approve via AskUserQuestion.
    """
    if not _any_bash_pattern_matches(cmd, allow_patterns):
        return "ask"
    if _any_bash_pattern_matches(cmd, deny_patterns):
        return "ask"
    return "auto-run"
