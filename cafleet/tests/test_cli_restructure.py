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


class TestAgentListHandlerName:
    """Round-8 rename: ``agent_list_`` → ``agent_list``.

    The trailing underscore was a leftover defense against shadowing
    Python's built-in ``list``; click's command name (``agent list``) is
    independent of the Python function name. Lock the consistent naming
    in so a future rename back to ``agent_list_`` would flip this test.
    """

    def test_function_named_agent_list(self):
        from cafleet.cli import agent_list

        # Click wraps decorated commands; the underlying callback's
        # ``__name__`` is the source-level function identifier.
        if hasattr(agent_list, "callback") and agent_list.callback is not None:
            assert agent_list.callback.__name__ == "agent_list"
        else:
            assert agent_list.__name__ == "agent_list"

    def test_old_name_no_longer_exported(self):
        import cafleet.cli as cli_mod

        assert not hasattr(cli_mod, "agent_list_"), (
            "stale ``agent_list_`` symbol survived the rename"
        )
