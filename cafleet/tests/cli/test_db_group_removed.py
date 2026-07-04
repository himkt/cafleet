"""Regression guard: the ``cafleet db`` CLI group no longer exists.

This file is the named carve-out for the design 0000117 full-corpus grep —
it embeds the legacy invocation it asserts is rejected. Absence is asserted
via Click's standard unknown-command error, not a deprecation shim.
"""

import importlib

import pytest
from click.testing import CliRunner

from cafleet.cli import cli


def test_db_group_removed__no_such_command_exit_2():
    runner = CliRunner()
    result = runner.invoke(cli, ["db", "init"])
    assert result.exit_code == 2, result.output
    assert "No such command 'db'." in (result.output or "")


def test_db_group_removed__cli_db_module_gone():
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module("cafleet.cli.db")
