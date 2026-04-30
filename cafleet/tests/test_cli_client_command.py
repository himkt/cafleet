"""Tests for the ``_client_command`` decorator (design 0000041 §A.1).

The Programmer adds a private decorator ``_client_command`` to ``cli.py``
that subsumes the ``--session-id`` guard, optional ``--agent-id``-belongs-to-session
validation, broker-error wrapping, and JSON-vs-text output branching.

The tests use a tiny test-only click group (declared at module top) wired
to the decorator so we exercise it end-to-end via ``CliRunner`` without
depending on any of the migrated production commands.
"""

import json

import click
import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import _client_command


@click.group()
@click.option("--json", "json_output", is_flag=True, default=False)
@click.option("--session-id", "session_id", default=None)
@click.pass_context
def _test_cli(ctx, json_output, session_id):
    ctx.ensure_object(dict)
    ctx.obj["session_id"] = session_id
    ctx.obj["json_output"] = json_output


@_test_cli.command("simple")
@click.pass_context
@_client_command(text_formatter=lambda r: f"TEXT:{r}")
def _simple(ctx):
    return {"hello": "world"}


@_test_cli.command("agent-bound")
@click.option("--agent-id", required=True)
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda r: f"TEXT:{r}",
)
def _agent_bound(ctx, agent_id):
    return {"ok": True, "agent_id": agent_id}


@_test_cli.command("raises")
@click.pass_context
@_client_command()
def _raises(ctx):
    raise RuntimeError("boom!")


@pytest.fixture
def runner():
    return CliRunner()


class TestSessionIdGuard:
    def test_missing_session_id_raises_click_exception(self, runner):
        result = runner.invoke(_test_cli, ["simple"])
        assert result.exit_code != 0
        assert "session-id" in result.output.lower() or "is required" in result.output


class TestRequiresAgentSession:
    def test_false_does_not_call_verify(self, runner, monkeypatch):
        verify_calls = []

        def fake_verify(aid, sid):
            verify_calls.append((aid, sid))
            return True

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)

        result = runner.invoke(
            _test_cli,
            ["--session-id", "session-1", "simple"],
        )
        assert result.exit_code == 0, result.output
        assert verify_calls == []

    def test_true_calls_verify_and_raises_on_false(self, runner, monkeypatch):
        verify_calls = []

        def fake_verify(aid, sid):
            verify_calls.append((aid, sid))
            return False

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)

        result = runner.invoke(
            _test_cli,
            [
                "--session-id",
                "session-1",
                "agent-bound",
                "--agent-id",
                "agent-1",
            ],
        )
        assert result.exit_code != 0
        assert "not a member of session" in result.output
        assert verify_calls == [("agent-1", "session-1")]

    def test_true_proceeds_when_verify_returns_true(self, runner, monkeypatch):
        monkeypatch.setattr(broker, "verify_agent_session", lambda _a, _s: True)

        result = runner.invoke(
            _test_cli,
            [
                "--session-id",
                "session-1",
                "agent-bound",
                "--agent-id",
                "agent-1",
            ],
        )
        assert result.exit_code == 0, result.output
        assert "TEXT:" in result.output
        assert "agent-1" in result.output


class TestBrokerErrorWrapping:
    def test_runtime_error_wrapped_as_click_exception(self, runner):
        result = runner.invoke(
            _test_cli,
            ["--session-id", "session-1", "raises"],
        )
        assert result.exit_code == 1, result.output
        assert "boom!" in result.output


class TestOutputBranching:
    def test_json_output_branch_uses_format_json(self, runner):
        result = runner.invoke(
            _test_cli,
            ["--json", "--session-id", "session-1", "simple"],
        )
        assert result.exit_code == 0, result.output
        parsed = json.loads(result.output)
        assert parsed == {"hello": "world"}

    def test_text_output_branch_uses_text_formatter(self, runner):
        result = runner.invoke(
            _test_cli,
            ["--session-id", "session-1", "simple"],
        )
        assert result.exit_code == 0, result.output
        assert result.output.startswith("TEXT:")
        assert "hello" in result.output
        assert "world" in result.output
