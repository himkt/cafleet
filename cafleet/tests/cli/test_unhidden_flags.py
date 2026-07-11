"""Regression tests: the shared operator flags are visible and documented (design 0000118, items 3.4 + 3.5).

``--full`` / ``--quiet`` (shared ``full_flag`` / ``quiet_flag``), ``--activity``
(``member list``), and ``--ansi/--no-ansi`` (``member capture``) are visible
and carry help text, so they appear in the relevant ``--help`` output. The
Click param carries ``hidden is False`` and a non-empty ``help``.
"""

import pytest
from click.testing import CliRunner

from cafleet.cli import cli

# (command path, Click param name) for every flag covered by this guard.
_UNHIDDEN_FLAGS = [
    (("message", "send"), "full"),
    (("message", "send"), "quiet"),
    (("message", "ack"), "quiet"),
    (("member", "ping"), "quiet"),
    (("member", "list"), "activity"),
    (("member", "capture"), "ansi"),
]


@pytest.fixture
def runner():
    return CliRunner()


def _param(path, param_name):
    command = cli
    for name in path:
        command = command.commands[name]
    for param in command.params:
        if param.name == param_name:
            return param
    raise AssertionError(f"no param {param_name!r} on {' '.join(path)}")


@pytest.mark.parametrize(("path", "param_name"), _UNHIDDEN_FLAGS)
def test_flag_is_visible_and_documented(path, param_name):
    param = _param(path, param_name)
    assert param.hidden is False
    assert param.help


def test_message_send_help_shows_full_and_quiet(runner):
    result = runner.invoke(cli, ["message", "send", "--help"])
    assert result.exit_code == 0, result.output
    assert "--full" in result.output
    assert "--quiet" in result.output


def test_member_list_help_shows_activity(runner):
    result = runner.invoke(cli, ["member", "list", "--help"])
    assert result.exit_code == 0, result.output
    assert "--activity" in result.output


def test_member_capture_help_shows_ansi(runner):
    result = runner.invoke(cli, ["member", "capture", "--help"])
    assert result.exit_code == 0, result.output
    assert "--ansi" in result.output
    assert "--no-ansi" in result.output
