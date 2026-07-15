"""Tests for the per-subcommand ``cafleet <subcommand> --fleet-id <int>`` CLI option."""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from tests._helpers import _init_registry


@pytest.fixture
def db_runner(tmp_path, monkeypatch):
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    runner = CliRunner()
    _init_registry()
    return runner


def test_missing_fleet_id_fails_client_subcommands__member_list_without_fleet_id_shows_new_error_message(
    db_runner,
):
    result = db_runner.invoke(cli, ["member", "list"])
    out = result.output or ""
    assert "--fleet-id" in out
    assert "is required" in out
    assert "cafleet fleet create" in out
    assert "CAFLEET_FLEET_ID" not in out
    assert "environment variable" not in out.lower()


def test_fleet_id_flag_flows_into_broker__member_list_passes_fleet_id_to_broker(
    db_runner, monkeypatch
):
    captured: dict = {}

    def fake_list_members(*args, **kwargs):
        captured["args"] = args
        captured["kwargs"] = kwargs
        return []

    monkeypatch.setattr(broker, "list_members", fake_list_members)

    sid = 100
    result = db_runner.invoke(
        cli,
        [
            "member",
            "list",
            "--fleet-id",
            str(sid),
        ],
    )

    assert result.exit_code == 0, result.output
    all_values = list(captured["args"]) + list(captured["kwargs"].values())
    assert sid in all_values


def test_fleet_id_flag_flows_into_broker__send_passes_fleet_id_to_broker(
    db_runner, monkeypatch
):
    captured: dict = {}

    def fake_send_message(*args, **kwargs):
        captured["args"] = args
        captured["kwargs"] = kwargs
        sender = args[1] if len(args) > 1 else kwargs.get("from_member_id")
        recipient = args[2] if len(args) > 2 else kwargs.get("to_member_id")
        return {
            "message": {
                "message_id": 5000,
                "owner_member_id": recipient,
                "from_member_id": sender,
                "to_member_id": recipient,
                "type": "unicast",
                "created_at": "2026-01-01T00:00:00+00:00",
                "status_state": "input_required",
                "status_timestamp": "2026-01-01T00:00:00+00:00",
                "origin_message_id": None,
                "text": "hi",
            }
        }

    monkeypatch.setattr(broker, "send_message", fake_send_message)
    monkeypatch.setattr(broker, "verify_member_fleet", lambda *a, **k: True)

    sid = 100
    from_id = 200
    to_id = 300
    result = db_runner.invoke(
        cli,
        [
            "message",
            "send",
            "--fleet-id",
            str(sid),
            "--from-member-id",
            str(from_id),
            "--to-member-id",
            str(to_id),
            "--text",
            "hi",
        ],
    )

    assert result.exit_code == 0, result.output
    all_values = list(captured["args"]) + list(captured["kwargs"].values())
    assert sid in all_values


def test_fleet_id_flag_flows_into_broker__fleet_id_not_read_from_environment(
    db_runner, monkeypatch
):
    """Fleet id is read only from the ``--fleet-id`` flag; the environment is never consulted."""
    monkeypatch.setenv("CAFLEET_FLEET_ID", "100")
    result = db_runner.invoke(cli, ["member", "list"])
    assert result.exit_code == 1, result.output


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_subcommands_that_do_not_require_fleet_id__fleet_create_without_fleet_id(
    db_runner,
):
    """fleet create mints a fleet, so it cannot itself require one."""
    result = db_runner.invoke(
        cli, ["fleet", "create", "--name", "smoke", "--coding-agent", "claude"]
    )
    assert result.exit_code == 0, result.output


def test_subcommands_that_do_not_require_fleet_id__fleet_list_without_fleet_id(
    db_runner,
):
    result = db_runner.invoke(cli, ["fleet", "list"])
    assert result.exit_code == 0, result.output


def test_fleet_id_rejected_where_not_required__fleet_create_rejects_in_both_positions(
    db_runner,
):
    """``fleet create`` rejects ``--fleet-id`` in both the old global position and
    the per-subcommand position (exit 2, 'no such option')."""
    sid = "100"
    global_pos = db_runner.invoke(
        cli, ["--fleet-id", sid, "fleet", "create", "--name", "x"]
    )
    assert global_pos.exit_code == 2, global_pos.output
    assert "no such option" in (global_pos.output or "").lower()

    per_subcommand = db_runner.invoke(
        cli, ["fleet", "create", "--fleet-id", sid, "--name", "x"]
    )
    assert per_subcommand.exit_code == 2, per_subcommand.output
    assert "no such option" in (per_subcommand.output or "").lower()


def _create_fleet_via_cli(runner: CliRunner) -> int:
    """Run ``fleet create --json`` and return the fleet_id."""
    result = runner.invoke(
        cli,
        [
            "fleet",
            "create",
            "--name",
            "flag-test",
            "--coding-agent",
            "claude",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    return json.loads(result.output)["fleet_id"]


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_member_delete_cli__unknown_member_exits_nonzero(
    db_runner,
):
    fleet_id = _create_fleet_via_cli(db_runner)
    bogus_member_id = 999999

    result = db_runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(bogus_member_id),
        ],
    )
    assert result.exit_code == 1, result.output
    assert f"Member {bogus_member_id} not found" in (result.output or "")


def test_old_surface_removed__session_flag_and_group_rejected(db_runner):
    """Absence guard: no ``--session-id`` flag and no ``session`` command group
    exist — Click rejects both."""
    flag = db_runner.invoke(cli, ["--session-id", "100", "fleet", "list"])
    assert flag.exit_code == 2
    assert "no such option" in (flag.output or "").lower()

    group = db_runner.invoke(cli, ["session", "create"])
    assert group.exit_code == 2
    assert "no such command" in (group.output or "").lower()


def test_old_surface_removed__global_fleet_id_rejected(db_runner):
    """Absence guard: ``--fleet-id`` is a per-subcommand option only — Click
    rejects the flag in the global position with its standard 'no such option'
    error (exit 2)."""
    result = db_runner.invoke(cli, ["--fleet-id", "100", "member", "list"])
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()
