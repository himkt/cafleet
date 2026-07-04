"""Tests for the per-subcommand ``cafleet <subcommand> --fleet-id <int>`` CLI option."""

import json
import sqlite3

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
        sender = args[1] if len(args) > 1 else kwargs.get("agent_id")
        recipient = args[2] if len(args) > 2 else kwargs.get("to")
        return {
            "task": {
                "task_id": 5000,
                "context_id": recipient,
                "from_agent_id": sender,
                "to_agent_id": recipient,
                "type": "unicast",
                "created_at": "2026-01-01T00:00:00+00:00",
                "status_state": "input_required",
                "status_timestamp": "2026-01-01T00:00:00+00:00",
                "origin_task_id": None,
                "text": "hi",
            }
        }

    monkeypatch.setattr(broker, "send_message", fake_send_message)
    monkeypatch.setattr(broker, "verify_agent_fleet", lambda *a, **k: True)

    sid = 100
    aid = 200
    bid = 300
    result = db_runner.invoke(
        cli,
        [
            "message",
            "send",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(aid),
            "--to",
            str(bid),
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
    result = db_runner.invoke(cli, ["fleet", "create", "--label", "smoke"])
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
        cli, ["--fleet-id", sid, "fleet", "create", "--label", "x"]
    )
    assert global_pos.exit_code == 2, global_pos.output
    assert "no such option" in (global_pos.output or "").lower()

    per_subcommand = db_runner.invoke(
        cli, ["fleet", "create", "--fleet-id", sid, "--label", "x"]
    )
    assert per_subcommand.exit_code == 2, per_subcommand.output
    assert "no such option" in (per_subcommand.output or "").lower()


def _create_fleet_via_cli(runner: CliRunner) -> tuple[int, int]:
    """Run ``fleet create --json`` and return (fleet_id, administrator_agent_id)."""
    result = runner.invoke(cli, ["fleet", "create", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    return data["fleet_id"], data["administrator_agent_id"]


def _fetch_agent_status(db_file, agent_id: str) -> tuple[str, str | None]:
    """Return (status, deregistered_at) for a given agent_id via raw SQLite."""
    conn = sqlite3.connect(str(db_file))
    try:
        row = conn.execute(
            "SELECT status, deregistered_at FROM agents WHERE agent_id = ?",
            (agent_id,),
        ).fetchone()
    finally:
        conn.close()
    assert row is not None, f"agent {agent_id} not found"
    return row[0], row[1]


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_deregister_administrator_cli_guard__cli_deregister_admin_exits_nonzero(
    db_runner,
):
    fleet_id, admin_id = _create_fleet_via_cli(db_runner)

    result = db_runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(admin_id),
        ],
    )
    assert result.exit_code == 1, result.output


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_deregister_administrator_cli_guard__cli_deregister_admin_message_is_user_friendly(
    db_runner,
):
    fleet_id, admin_id = _create_fleet_via_cli(db_runner)

    result = db_runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(admin_id),
        ],
    )
    out = result.output or ""
    assert "Administrator cannot be deregistered" in out
    assert "Traceback" not in out


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_deregister_administrator_cli_guard__cli_deregister_unknown_agent_exits_nonzero(
    db_runner,
):
    fleet_id, _admin_id = _create_fleet_via_cli(db_runner)
    bogus_agent_id = 999999

    result = db_runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(bogus_agent_id),
        ],
    )
    assert result.exit_code == 1, result.output
    assert f"Agent {bogus_agent_id} not found" in (result.output or "")


@pytest.mark.usefixtures("_mock_tmux_for_fleet_create")
def test_deregister_administrator_cli_guard__cli_deregister_admin_leaves_row_active(
    db_runner, tmp_path
):
    db_file = tmp_path / "cafleet.db"
    fleet_id, admin_id = _create_fleet_via_cli(db_runner)

    db_runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(admin_id),
        ],
    )
    status, deregistered_at = _fetch_agent_status(db_file, admin_id)
    assert status == "active"
    assert deregistered_at is None


def test_old_surface_removed__session_flag_and_group_no_longer_parse(db_runner):
    """Regression guard: the pre-rename ``--session-id`` flag and ``session``
    command group are gone — Click rejects both (testing the absence, not a
    deprecation shim)."""
    flag = db_runner.invoke(cli, ["--session-id", "100", "fleet", "list"])
    assert flag.exit_code == 2
    assert "no such option" in (flag.output or "").lower()

    group = db_runner.invoke(cli, ["session", "create"])
    assert group.exit_code == 2
    assert "no such command" in (group.output or "").lower()


def test_old_surface_removed__global_fleet_id_no_longer_parses(db_runner):
    """Regression guard: ``--fleet-id`` is no longer a global option, so the old
    surface (flag before the subcommand) is rejected by Click with its standard
    'no such option' error (exit 2)."""
    result = db_runner.invoke(cli, ["--fleet-id", "100", "member", "list"])
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()
