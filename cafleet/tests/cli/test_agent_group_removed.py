"""Regression guard: the ``cafleet agent`` CLI group no longer exists.

This file is the named carve-out for the design 0000116 full-corpus grep —
it embeds the legacy invocation it asserts is rejected. Absence is asserted
via Click's standard unknown-command error, not a deprecation shim.
"""

import importlib

import pytest
from click.testing import CliRunner

from cafleet.cli import cli


def test_agent_group_removed__no_such_command_exit_2():
    runner = CliRunner()
    result = runner.invoke(cli, ["agent", "list", "--fleet-id", "1"])
    assert result.exit_code == 2, result.output
    assert "No such command 'agent'." in (result.output or "")


def test_agent_group_removed__cli_agent_module_gone():
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module("cafleet.cli.agent")
