"""Tests for the ``client_command`` decorator.

``cli/_helpers.py`` provides a shared decorator ``client_command`` that subsumes
optional ``--member-id``-belongs-to-fleet validation (``member_kwarg`` names the
acting-member kwarg the fleet gate reads), broker-error wrapping, and
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
@click.option("--full", is_flag=True, default=False)
@click.pass_context
@client_command(text_formatter=lambda r, *, full=False: f"TEXT:{r}")
def _simple(ctx, full):
    return {"hello": "world"}


@_test_cli.command("member-bound")
@fleet_id_option
@click.option("--member-id", "member_id", type=int, required=True)
@click.option("--full", is_flag=True, default=False)
@click.pass_context
@client_command(
    requires_member_fleet=True,
    text_formatter=lambda r, *, full=False: f"TEXT:{r}",
)
def _member_bound(ctx, member_id, full):
    return {"ok": True, "member_id": member_id}


@_test_cli.command("sender-bound")
@fleet_id_option
@click.option("--from-member-id", "from_member_id", type=int, required=True)
@click.option("--full", is_flag=True, default=False)
@click.pass_context
@client_command(
    requires_member_fleet=True,
    member_kwarg="from_member_id",
    text_formatter=lambda r, *, full=False: f"TEXT:{r}",
)
def _sender_bound(ctx, from_member_id, full):
    return {"ok": True, "from_member_id": from_member_id}


@_test_cli.command("raises")
@fleet_id_option
@click.pass_context
@client_command(text_formatter=lambda r, *, full=False: f"TEXT:{r}")
def _raises(ctx):
    raise RuntimeError("boom!")


@pytest.fixture
def runner():
    return CliRunner()


def test_fleet_id_guard__missing_fleet_id_raises_click_exception(runner):
    result = runner.invoke(_test_cli, ["simple"])
    assert result.exit_code != 0
    assert "fleet-id" in result.output.lower() or "is required" in result.output


def test_requires_member_fleet__false_does_not_call_verify(runner, monkeypatch):
    verify_calls = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)

    result = runner.invoke(
        _test_cli,
        ["simple", "--fleet-id", "1"],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == []


def test_requires_member_fleet__true_calls_verify_and_raises_on_false(
    runner, monkeypatch
):
    verify_calls = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return False

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)

    result = runner.invoke(
        _test_cli,
        [
            "member-bound",
            "--fleet-id",
            "1",
            "--member-id",
            "7",
        ],
    )
    assert result.exit_code != 0
    assert "member 7 is not in fleet 1." in result.output
    assert verify_calls == [(7, 1)]


def test_requires_member_fleet__true_proceeds_when_verify_returns_true(
    runner, monkeypatch
):
    monkeypatch.setattr(broker, "verify_member_fleet", lambda _m, _s: True)

    result = runner.invoke(
        _test_cli,
        [
            "member-bound",
            "--fleet-id",
            "1",
            "--member-id",
            "7",
        ],
    )
    assert result.exit_code == 0, result.output
    assert "TEXT:" in result.output
    assert "7" in result.output


def test_requires_member_fleet__member_kwarg_reads_from_member_id(runner, monkeypatch):
    verify_calls = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return False

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)

    result = runner.invoke(
        _test_cli,
        [
            "sender-bound",
            "--fleet-id",
            "1",
            "--from-member-id",
            "9",
        ],
    )
    assert result.exit_code != 0
    assert "member 9 is not in fleet 1." in result.output
    assert verify_calls == [(9, 1)]


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
