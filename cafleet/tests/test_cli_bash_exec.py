"""Tests for ``cafleet bash-exec`` (Step 4 task 3, round-5c-era placement).

Round-5c-era file name and CLI invocation strings. Step 14 task 1 (round 6)
will rename this file to ``test_cli_member_exec.py`` and rewrite every
``cafleet bash-exec`` invocation inside to ``cafleet member exec``.

Per the canonical-status rule (§3 ``bash_result`` payload subsection):
``status`` is the sole source of truth for outcome branching. Every
assertion below switches on ``status``; ``exit_code`` is asserted ONLY for
``status == "ran"``.

Helper-process exit code is 0 for every payload outcome (ran / denied /
timeout) — the JSON output IS the result. Non-zero only on Click UsageError.
"""

import json

from click.testing import CliRunner

from cafleet.cli import cli


def _invoke(args: list[str]) -> tuple[int, dict]:
    """Invoke ``cafleet bash-exec`` and return (exit_code, parsed_json)."""
    runner = CliRunner()
    result = runner.invoke(cli, ["bash-exec", *args])
    # The helper always writes exactly one JSON object to stdout.
    return result.exit_code, json.loads(result.output)


class TestBashExecHappyPath:
    def test_echo_returns_status_ran_with_stdout_and_zero_exit_code(self):
        exit_code, payload = _invoke(["--cmd", "echo hello"])
        assert exit_code == 0
        assert payload["status"] == "ran"
        assert payload["exit_code"] == 0
        assert "hello" in payload["stdout"]
        assert payload["stderr"] == ""
        assert isinstance(payload["duration_ms"], int)
        assert payload["duration_ms"] >= 0


class TestBashExecTimeout:
    """Real ``sleep`` invocation — the test exercises ``subprocess.run``'s
    timeout path including the SIGKILL fallback. Mocking would defeat the
    purpose: timeout behavior IS what's under test.
    """

    def test_sleep_exceeding_timeout_returns_status_timeout(self):
        exit_code, payload = _invoke(["--cmd", "sleep 60", "--timeout", "1"])
        assert exit_code == 0
        assert payload["status"] == "timeout"
        # Per the canonical-status rule, do NOT assert on exit_code for
        # status != "ran". The internal value is documented as opaque.
        assert "hard-killed at 1 seconds." in payload["stderr"]


class TestBashExecTruncation:
    """Stdout > 64 KiB is truncated; the marker carries the verbatim
    original byte count and the 65536 last-bytes-shown count.
    """

    def test_stdout_over_64kib_is_truncated_with_exact_marker(self):
        # 200000 ``x`` bytes via python -c keeps the test deterministic.
        cmd = 'python -c "import sys; sys.stdout.write(\\"x\\" * 200000)"'
        exit_code, payload = _invoke(["--cmd", cmd])
        assert exit_code == 0
        assert payload["status"] == "ran"
        assert payload["exit_code"] == 0
        assert payload["stdout"].endswith(
            "\n[truncated: original was 200000 bytes; last 65536 bytes shown]\n"
        )
        # The captured-bytes portion should be exactly 65536 ``x`` characters
        # before the marker. Compute body length and verify upper bound.
        marker_start = payload["stdout"].index(
            "\n[truncated: original was 200000 bytes; last 65536 bytes shown]\n"
        )
        body = payload["stdout"][:marker_start]
        assert len(body) == 65536
        assert set(body) == {"x"}


class TestBashExecStdinPropagation:
    def test_cat_with_stdin_echoes_input_to_stdout(self):
        exit_code, payload = _invoke(["--cmd", "cat", "--stdin", "hello world"])
        assert exit_code == 0
        assert payload["status"] == "ran"
        assert payload["exit_code"] == 0
        assert "hello world" in payload["stdout"]


class TestBashExecEmptyCmdDenied:
    """§3 helper bullet 1: empty cmd short-circuits with a denied JSON
    object on stdout. Helper-process exit code is 0 (the validation
    failure is a payload-level outcome, not a CLI-arg error).
    """

    def test_empty_cmd_returns_status_denied_with_verbatim_reason(self):
        exit_code, payload = _invoke(["--cmd", ""])
        assert exit_code == 0
        assert payload["status"] == "denied"
        # Per the canonical-status rule, do NOT assert on exit_code.
        assert "bash_request.cmd may not be empty." in payload["stderr"]
        assert payload["stdout"] == ""
        assert payload["duration_ms"] == 0


class TestBashExecOverCapTimeoutDenied:
    """§3 helper bullet 1: timeout > 600 short-circuits with a denied JSON
    object on stdout. Helper-process exit code is 0.
    """

    def test_over_cap_timeout_returns_status_denied_with_verbatim_reason(self):
        exit_code, payload = _invoke(["--cmd", "echo x", "--timeout", "9999"])
        assert exit_code == 0
        assert payload["status"] == "denied"
        # Per the canonical-status rule, do NOT assert on exit_code.
        assert "bash_request.timeout exceeds 600s cap." in payload["stderr"]
        assert payload["stdout"] == ""
        assert payload["duration_ms"] == 0


class TestBashExecNonexistentCwd:
    """§12 edge case: ``cwd`` does not exist → runtime path, not denied.

    The helper invokes the shell which raises FileNotFoundError;
    ``status: "ran"`` with non-zero ``exit_code`` is the closest fit.
    Helper-process exit code is 0; the failure surfaces in the payload.
    """

    def test_nonexistent_cwd_returns_status_ran_nonzero_exit_with_stderr(self):
        exit_code, payload = _invoke(["--cmd", "echo x", "--cwd", "/no/such/dir"])
        assert exit_code == 0
        assert payload["status"] == "ran"
        # exit_code is meaningful for status == "ran". Asserting non-zero
        # is enough — the exact value depends on how the runtime surfaces
        # FileNotFoundError (subprocess returncode varies by platform).
        assert payload["exit_code"] != 0
        assert "no such cwd" in payload["stderr"].lower() or (
            "/no/such/dir" in payload["stderr"]
        )
