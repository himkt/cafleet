"""Tests for ``cafleet.bash_routing`` payload helpers (Step 4 task 4).

The design doc §11 specifies ``parse_bash_request`` and ``format_bash_result``
plus a ``BashRequest`` / ``BashResult`` pair. The doc explicitly leaves the
``BashRequest`` / ``BashResult`` shape as "dataclass or TypedDict" — these
tests treat the parsed result as a mapping (dict subscript), which works for
both ``TypedDict`` (a ``dict`` at runtime) and ``dataclass`` (via
``dataclasses.asdict``). If the implementation picks dataclass, this file
must convert through ``dataclasses.asdict`` before subscript access; until
then the simpler mapping contract is asserted.

Truncation-marker formatting lives in ``cafleet member exec`` (the helper),
NOT in ``format_bash_result`` — see ``test_cli_bash_exec.py`` for that path.
``format_bash_result`` is a pure formatter; callers pass already-truncated
streams.
"""

import json

from cafleet.bash_routing import format_bash_result, parse_bash_request


def _as_mapping(parsed):
    """Normalize ``parse_bash_request`` output to a mapping for subscript access.

    The implementation may return a ``TypedDict`` (already a mapping) or a
    ``dataclass`` instance. This helper accepts either.
    """
    if parsed is None:
        return None
    if isinstance(parsed, dict):
        return parsed
    # Fallback for dataclass instances.
    from dataclasses import asdict, is_dataclass

    if is_dataclass(parsed):
        return asdict(parsed)
    raise TypeError(f"Unexpected parse_bash_request return type: {type(parsed)!r}")


class TestParseBashRequestRoundTrip:
    def test_well_formed_payload_returns_all_fields(self):
        text = json.dumps(
            {
                "type": "bash_request",
                "cmd": "git log -1 --oneline",
                "cwd": "/home/himkt/work/himkt/cafleet",
                "stdin": None,
                "timeout": 30,
                "reason": "verifying main before PR",
            }
        )
        parsed = _as_mapping(parse_bash_request(text))
        assert parsed is not None
        assert parsed["type"] == "bash_request"
        assert parsed["cmd"] == "git log -1 --oneline"
        assert parsed["cwd"] == "/home/himkt/work/himkt/cafleet"
        assert parsed["stdin"] is None
        assert parsed["timeout"] == 30
        assert parsed["reason"] == "verifying main before PR"

    def test_payload_with_stdin_and_full_fields(self):
        text = json.dumps(
            {
                "type": "bash_request",
                "cmd": "cat",
                "cwd": "/tmp",
                "stdin": "hello world",
                "timeout": 60,
                "reason": "testing stdin propagation",
            }
        )
        parsed = _as_mapping(parse_bash_request(text))
        assert parsed is not None
        assert parsed["cmd"] == "cat"
        assert parsed["stdin"] == "hello world"
        assert parsed["timeout"] == 60


class TestParseBashRequestNonBashRequestShapes:
    """Per the §11 / Step 4 task 4 docstring: ``parse_bash_request`` returns
    ``None`` for non-``bash_request`` shapes (parse fail / missing type / type
    mismatch). It does NOT raise.
    """

    def test_invalid_json_returns_none(self):
        assert parse_bash_request("not-json{") is None

    def test_empty_string_returns_none(self):
        assert parse_bash_request("") is None

    def test_json_array_returns_none(self):
        # Top-level must be an object with a ``type`` key.
        assert parse_bash_request("[1, 2, 3]") is None

    def test_json_string_returns_none(self):
        assert parse_bash_request('"just a string"') is None

    def test_missing_type_returns_none(self):
        text = json.dumps(
            {
                "cmd": "git log -1",
                "cwd": "/tmp",
                "reason": "test",
            }
        )
        assert parse_bash_request(text) is None

    def test_type_mismatch_returns_none(self):
        text = json.dumps(
            {
                "type": "wat",
                "cmd": "git log -1",
                "cwd": "/tmp",
                "reason": "test",
            }
        )
        assert parse_bash_request(text) is None

    def test_bash_result_type_returns_none(self):
        # Replies are NOT bash_requests; the parser is a discriminator only.
        text = json.dumps(
            {
                "type": "bash_result",
                "in_reply_to": "task-001",
                "status": "ran",
                "exit_code": 0,
                "stdout": "",
                "stderr": "",
                "duration_ms": 0,
            }
        )
        assert parse_bash_request(text) is None


class TestParseBashRequestDoesNotValidateFieldSemantics:
    """Per the §11 / Step 4 task 4 docstring: empty ``cmd`` and oversized
    ``timeout`` are NOT rejected by ``parse_bash_request``. The helper
    (``cafleet member exec``) is the input-validation point. The parser's
    job is purely shape-discrimination.
    """

    def test_empty_cmd_returned_verbatim(self):
        text = json.dumps(
            {
                "type": "bash_request",
                "cmd": "",
                "cwd": "/tmp",
                "reason": "intentionally empty",
            }
        )
        parsed = _as_mapping(parse_bash_request(text))
        assert parsed is not None
        assert parsed["cmd"] == ""

    def test_oversized_timeout_returned_verbatim(self):
        text = json.dumps(
            {
                "type": "bash_request",
                "cmd": "echo hi",
                "cwd": "/tmp",
                "timeout": 9999,
                "reason": "intentionally too long",
            }
        )
        parsed = _as_mapping(parse_bash_request(text))
        assert parsed is not None
        assert parsed["timeout"] == 9999


class TestFormatBashResultRoundTrip:
    """``format_bash_result`` returns a JSON-encoded string suitable for
    ``cafleet message send --text``. Round-trip via ``json.loads``.
    """

    def test_status_ran_includes_exit_code_verbatim(self):
        text = format_bash_result(
            in_reply_to="task-001",
            status="ran",
            exit_code=0,
            stdout="hello\n",
            stderr="",
            duration_ms=47,
        )
        decoded = json.loads(text)
        assert decoded["type"] == "bash_result"
        assert decoded["in_reply_to"] == "task-001"
        assert decoded["status"] == "ran"
        assert decoded["exit_code"] == 0
        assert decoded["stdout"] == "hello\n"
        assert decoded["stderr"] == ""
        assert decoded["duration_ms"] == 47

    def test_status_ran_with_nonzero_exit_code(self):
        text = format_bash_result(
            in_reply_to="task-002",
            status="ran",
            exit_code=42,
            stdout="",
            stderr="boom\n",
            duration_ms=12,
        )
        decoded = json.loads(text)
        assert decoded["status"] == "ran"
        assert decoded["exit_code"] == 42
        assert decoded["stderr"] == "boom\n"

    def test_status_denied_serializes_caller_provided_exit_code_opaquely(self):
        # Per the canonical-status rule, ``exit_code`` is opaque on denied
        # outcomes — but the formatter must still serialize whatever the
        # caller passes (clients switch on ``status``, never ``exit_code``).
        text = format_bash_result(
            in_reply_to="task-003",
            status="denied",
            exit_code=999,
            stdout="",
            stderr="Director denied the request.",
            duration_ms=0,
        )
        decoded = json.loads(text)
        assert decoded["status"] == "denied"
        assert decoded["exit_code"] == 999  # Caller-opaque, but serialized.
        assert decoded["stderr"] == "Director denied the request."
        assert decoded["duration_ms"] == 0

    def test_status_timeout_serializes_caller_provided_exit_code_opaquely(self):
        text = format_bash_result(
            in_reply_to="task-004",
            status="timeout",
            exit_code=124,
            stdout="partial\n",
            stderr="hard-killed at 1 seconds.",
            duration_ms=1000,
        )
        decoded = json.loads(text)
        assert decoded["status"] == "timeout"
        # The internal value is 124 for shell-legibility, but per the
        # canonical-status rule clients must switch on ``status``.
        assert decoded["exit_code"] == 124
        assert "hard-killed at 1 seconds." in decoded["stderr"]


class TestFormatBashResultNoteField:
    def test_note_present_when_passed(self):
        text = format_bash_result(
            in_reply_to="task-005",
            status="ran",
            exit_code=0,
            stdout="abc\n",
            stderr="",
            duration_ms=5,
            note="ran without operator prompt (matched allow rule: Bash(git *))",
        )
        decoded = json.loads(text)
        assert "note" in decoded
        assert decoded["note"] == (
            "ran without operator prompt (matched allow rule: Bash(git *))"
        )

    def test_note_absent_when_default_none(self):
        # Per design §3 ``bash_result`` payload table: ``note`` is "Omitted
        # otherwise." When the caller does not pass a ``note``, the formatter
        # must NOT emit a ``note: null`` key.
        text = format_bash_result(
            in_reply_to="task-006",
            status="ran",
            exit_code=0,
            stdout="ok\n",
            stderr="",
            duration_ms=3,
        )
        decoded = json.loads(text)
        assert "note" not in decoded


class TestFormatBashResultDefaults:
    """Default kwargs produce a minimal valid payload.

    ``stdout``, ``stderr`` default to ``""``; ``duration_ms`` defaults to 0;
    ``note`` defaults to absent. ``in_reply_to``, ``status``, ``exit_code``
    are required.
    """

    def test_minimal_payload_with_defaults(self):
        text = format_bash_result(
            in_reply_to="task-min",
            status="denied",
            exit_code=126,
        )
        decoded = json.loads(text)
        assert decoded["type"] == "bash_result"
        assert decoded["in_reply_to"] == "task-min"
        assert decoded["status"] == "denied"
        assert decoded["exit_code"] == 126
        assert decoded["stdout"] == ""
        assert decoded["stderr"] == ""
        assert decoded["duration_ms"] == 0
        assert "note" not in decoded


class TestFormatBashResultPreservesTruncationMarkerVerbatim:
    """The truncation marker is produced upstream in the helper. The
    formatter MUST NOT mangle it — it embeds the already-truncated stream
    as-is.
    """

    def test_truncation_marker_in_stdout_passes_through(self):
        marker = "\n[truncated: original was 200000 bytes; last 65536 bytes shown]\n"
        text = format_bash_result(
            in_reply_to="task-trunc",
            status="ran",
            exit_code=0,
            stdout="abc" + marker,
            stderr="",
            duration_ms=10,
        )
        decoded = json.loads(text)
        assert decoded["stdout"].endswith(marker)
        assert "[truncated: original was 200000 bytes; last 65536 bytes shown]" in (
            decoded["stdout"]
        )

    def test_truncation_marker_in_stderr_passes_through(self):
        marker = "\n[truncated: original was 99999 bytes; last 65536 bytes shown]\n"
        text = format_bash_result(
            in_reply_to="task-trunc-err",
            status="ran",
            exit_code=1,
            stdout="",
            stderr="oops" + marker,
            duration_ms=4,
        )
        decoded = json.loads(text)
        assert decoded["stderr"].endswith(marker)
