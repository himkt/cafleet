"""Regression guard: every flat-verb subcommand removed in the round-6
nested-only restructure (design 0000034 §14) MUST fail with Click's default
``Error: No such command '<name>'.`` (exit code 2 — UsageError).

If a future contributor accidentally re-adds a Click alias for any of these
flat-verb names (e.g. via ``@cli.command("send")``), one of these tests
flips to passing the wrong way and the regression guard catches the drift.
"""

import pytest
from click.testing import CliRunner

from cafleet.cli import cli

REMOVED_FLAT_VERBS = [
    "send",
    "poll",
    "ack",
    "cancel",
    "broadcast",
    "register",
    "deregister",
    "agents",
    "get-task",
    "bash-exec",
]


class TestFlatVerbsRejected:
    @pytest.mark.parametrize("verb", REMOVED_FLAT_VERBS)
    def test_flat_verb_no_longer_parses(self, verb):
        runner = CliRunner()
        result = runner.invoke(cli, [verb])
        assert result.exit_code == 2, (
            f"Expected exit 2 (UsageError) for removed flat verb '{verb}'; "
            f"got {result.exit_code}. Output: {result.output!r}"
        )
        out = result.output or ""
        assert "No such command" in out, (
            f"Expected 'No such command' in output for '{verb}'; got: {out!r}"
        )
        assert verb in out, f"Expected verb name '{verb}' in error output; got: {out!r}"
