"""Regression guard: the removed identity-flag spellings no longer parse.

This file is the named carve-out for the removed registry-noun flags — it
embeds the legacy invocations it asserts are rejected. Absence is asserted
via Click's standard no-such-option error, not a deprecation shim.
"""

from click.testing import CliRunner

from cafleet.cli import cli


def test_message_send_agent_id_removed__no_such_option_exit_2():
    runner = CliRunner()
    result = runner.invoke(
        cli,
        [
            "message",
            "send",
            "--fleet-id",
            "1",
            "--agent-id",
            "1",
            "--to-member-id",
            "2",
            "--text",
            "hi",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "No such option: --agent-id" in (result.output or "")


def test_message_send_to_removed__no_such_option_exit_2():
    runner = CliRunner()
    result = runner.invoke(
        cli,
        [
            "message",
            "send",
            "--fleet-id",
            "1",
            "--from-member-id",
            "1",
            "--to",
            "2",
            "--text",
            "hi",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "No such option: --to" in (result.output or "")


def test_member_create_agent_id_removed__no_such_option_exit_2():
    runner = CliRunner()
    result = runner.invoke(
        cli,
        [
            "member",
            "create",
            "--fleet-id",
            "1",
            "--agent-id",
            "1",
            "--name",
            "worker",
            "--description",
            "a worker",
            "--text",
            "prompt",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "No such option: --agent-id" in (result.output or "")
