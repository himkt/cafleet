"""Tests for the ``cafleet --version`` global CLI flag (design 0000031)."""

from importlib.metadata import version

from click.testing import CliRunner

from cafleet.cli import cli


def test_version_flag_prints_cafleet_and_version() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0, result.output
    assert result.output == f"cafleet {version('cafleet')}\n", result.output


def test_version_flag_does_not_require_session_id() -> None:
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0, result.output
    assert "session-id" not in result.output.lower(), result.output
