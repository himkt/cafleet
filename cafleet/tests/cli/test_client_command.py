"""Tests for the ``client_command`` decorator.

``cli/_helpers.py`` provides a shared decorator ``client_command`` that subsumes
optional ``--agent-id``-belongs-to-fleet validation, broker-error wrapping, and
JSON-vs-text output branching. The per-subcommand ``fleet_id_option`` decorator
enforces the ``--fleet-id`` requirement (custom message, ``type=int``) and stashes
the value into ``ctx.obj["fleet_id"]``.

The tests use a tiny test-only click group (declared at module top) wired to the
decorators so we exercise them end-to-end via ``CliRunner`` without depending on
any of the migrated production commands.
"""

import json

import click
import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli._helpers import client_command, fleet_id_option


@click.group()
@click.option("--json", "json_output", is_flag=True, default=False)
@click.pass_context
def _test_cli(ctx, json_output):
    ctx.ensure_object(dict)
    ctx.obj["json_output"] = json_output


@_test_cli.command("simple")
@fleet_id_option
@click.pass_context
@client_command(text_formatter=lambda r: f"TEXT:{r}")
def _simple(ctx):
    return {"hello": "world"}


@_test_cli.command("agent-bound")
@fleet_id_option
@click.option("--agent-id", required=True)
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r: f"TEXT:{r}",
)
def _agent_bound(ctx, agent_id):
    return {"ok": True, "agent_id": agent_id}


@_test_cli.command("raises")
@fleet_id_option
@click.pass_context
@client_command()
def _raises(ctx):
    raise RuntimeError("boom!")


@pytest.fixture
def runner():
    return CliRunner()


def test_fleet_id_guard__missing_fleet_id_raises_click_exception(runner):
    result = runner.invoke(_test_cli, ["simple"])
    assert result.exit_code != 0
    assert "fleet-id" in result.output.lower() or "is required" in result.output


def test_requires_agent_fleet__false_does_not_call_verify(runner, monkeypatch):
    verify_calls = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)

    result = runner.invoke(
        _test_cli,
        ["simple", "--fleet-id", "1"],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == []


def test_requires_agent_fleet__true_calls_verify_and_raises_on_false(
    runner, monkeypatch
):
    verify_calls = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return False

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)

    result = runner.invoke(
        _test_cli,
        [
            "agent-bound",
            "--fleet-id",
            "1",
            "--agent-id",
            "agent-1",
        ],
    )
    assert result.exit_code != 0
    assert "not a member of fleet" in result.output
    assert verify_calls == [("agent-1", 1)]


def test_requires_agent_fleet__true_proceeds_when_verify_returns_true(
    runner, monkeypatch
):
    monkeypatch.setattr(broker, "verify_agent_fleet", lambda _a, _s: True)

    result = runner.invoke(
        _test_cli,
        [
            "agent-bound",
            "--fleet-id",
            "1",
            "--agent-id",
            "agent-1",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "TEXT:" in result.output
    assert "agent-1" in result.output


def test_broker_error_wrapping__runtime_error_wrapped_as_click_exception(runner):
    result = runner.invoke(
        _test_cli,
        ["raises", "--fleet-id", "1"],
    )
    assert result.exit_code == 1, result.output
    assert "boom!" in result.output


def test_output_branching__json_output_branch_uses_format_json(runner):
    result = runner.invoke(
        _test_cli,
        ["--json", "simple", "--fleet-id", "1"],
    )
    assert result.exit_code == 0, result.output
    parsed = json.loads(result.output)
    assert parsed == {"hello": "world"}


def test_output_branching__text_output_branch_uses_text_formatter(runner):
    result = runner.invoke(
        _test_cli,
        ["simple", "--fleet-id", "1"],
    )
    assert result.exit_code == 0, result.output
    assert result.output.startswith("TEXT:")
    assert "hello" in result.output
    assert "world" in result.output
