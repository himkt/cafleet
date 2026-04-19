"""Tests for the ``cafleet --version`` global CLI flag (design 0000031)."""

from click.testing import CliRunner

from cafleet.cli import cli


def test_version_flag_prints_cafleet_and_version() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert result.output.startswith("cafleet ")
    assert any(ch.isdigit() for ch in result.output)
    assert result.output.endswith("\n")


def test_version_flag_does_not_require_session_id() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert "session-id" not in result.output.lower()
